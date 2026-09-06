//! deezer adapter: official metadata, session stream, youtube fallback

use anyhow::{Context, Result};
use reqwest::Client as Http;
use reqwest::header::{CONTENT_TYPE, COOKIE, ORIGIN, REFERER};
use serde_json::{Value, json};
use url::Url;

use crate::media::{Asset, Format, Kind, Media, Source, fmt_secs};

use super::spotify::rank;
use super::youtube;

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0 Safari/537.36";
const API: &str = "https://api.deezer.com";
const GW: &str = "https://www.deezer.com/ajax/gw-light.php";

pub struct Client {
    http: Http,
    arl: Option<String>,
    session: Option<Session>,
    yt: youtube::Api,
}

struct Session {
    sid: String,
    token: String,
}

impl Client {
    pub fn new(proxy: Option<&str>, arl: Option<String>) -> Result<Self> {
        let mut builder = Http::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent(UA);
        if let Some(p) = proxy {
            builder = builder.proxy(reqwest::Proxy::all(p).context("proxy")?);
        }
        Ok(Self {
            http: builder.build().context("client")?,
            arl,
            session: None,
            yt: youtube::Api::new(proxy)?,
        })
    }

    pub async fn track(&mut self, url: Url, format: Format) -> Result<Media> {
        let kind = kind(&url)?;
        let id = id_after(&url, &kind)?;
        let t = self.api(&format!("/{kind}/{id}")).await?;
        self.track_from(&t, format).await
    }

    pub async fn album(&mut self, url: Url, format: Format) -> Result<Vec<Media>> {
        self.collection(url, "album", format).await
    }

    pub async fn playlist(&mut self, url: Url, format: Format) -> Result<Vec<Media>> {
        self.collection(url, "playlist", format).await
    }
}

impl Client {
    async fn api(&self, path: &str) -> Result<Value> {
        let v: Value = self
            .http
            .get(format!("{API}{path}"))
            .send()
            .await?
            .json()
            .await
            .context("bad api json")?;
        if let Some(e) = v.get("error") {
            let msg = e
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("api error");
            anyhow::bail!("{msg}");
        }
        Ok(v)
    }

    async fn gw_call(
        &self,
        token: &str,
        sid: Option<&str>,
        method: &str,
        body: Value,
    ) -> Result<Value> {
        let mut url = Url::parse(GW)?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("api_version", "1.0");
            q.append_pair("api_token", token);
            q.append_pair("input", "3");
            q.append_pair("method", method);
        }
        let mut req = self
            .http
            .post(url)
            .header(CONTENT_TYPE, "application/json")
            .header(REFERER, "https://www.deezer.com/en/")
            .header(ORIGIN, "https://www.deezer.com")
            .json(&body);
        if let Some(sid) = sid {
            let mut cookie = format!("sid={sid}");
            if let Some(arl) = &self.arl {
                cookie.push_str(&format!("; arl={arl}"));
            }
            req = req.header(COOKIE, cookie);
        } else if let Some(arl) = &self.arl {
            req = req.header(COOKIE, format!("arl={arl}"));
        }
        let v: Value = req.send().await?.json().await.context("bad gw json")?;
        if let Some(e) = v.get("error") {
            let msg = e
                .as_object()
                .and_then(|o| o.values().next())
                .and_then(Value::as_str)
                .unwrap_or("gateway error");
            anyhow::bail!("deezer: {msg}");
        }
        Ok(v)
    }

    async fn bootstrap(&self) -> Result<Session> {
        let r = self.gw_call("null", None, "deezer.getUserData", json!({})).await?;
        let sid = r
            .pointer("/results/SESSION_ID")
            .and_then(Value::as_str)
            .context("no session")?;
        let token = r
            .pointer("/results/checkForm")
            .and_then(Value::as_str)
            .context("no check form")?;
        Ok(Session {
            sid: sid.into(),
            token: token.into(),
        })
    }

    async fn ensure_session(&mut self) -> Result<&Session> {
        if self.session.is_none() {
            self.session = Some(self.bootstrap().await?);
        }
        Ok(self.session.as_ref().expect("session"))
    }

    async fn song_data(&mut self, id: &str) -> Result<Value> {
        let sess = self.ensure_session().await?;
        let (sid, token) = (sess.sid.clone(), sess.token.clone());
        let r = self
            .gw_call(&token, Some(&sid), "song.getData", json!({ "SNG_ID": id }))
            .await?;
        Ok(r.get("results").cloned().unwrap_or(Value::Null))
    }

    async fn session_asset(&mut self, id: &str, format: Format) -> Result<Option<(String, String)>> {
        if self.arl.is_none() {
            return Ok(None);
        }
        let r = self.song_data(id).await?;
        if let Some((url, ext)) = r
            .get("MEDIA")
            .and_then(Value::as_array)
            .and_then(|media| pick_media(media, format))
        {
            return Ok(Some((url, ext)));
        }
        Ok(None)
    }

    async fn collection(&mut self, url: Url, kind: &str, format: Format) -> Result<Vec<Media>> {
        let id = id_after(&url, kind)?;
        let a = self.api(&format!("/{kind}/{id}")).await?;
        let tracks = a
            .pointer("/tracks/data")
            .and_then(Value::as_array)
            .context("no tracks")?
            .clone();
        let mut out = Vec::with_capacity(tracks.len());
        for t in &tracks {
            out.push(self.track_from(t, format).await?);
        }
        Ok(out)
    }

    async fn track_from(&mut self, t: &Value, format: Format) -> Result<Media> {
        let title = t
            .get("title")
            .and_then(Value::as_str)
            .context("no title")?
            .to_string();
        let artist = t
            .pointer("/artist/name")
            .or_else(|| t.pointer("/contributors/0/name"))
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let tags = tags_of(t);
        let want = t.get("duration").and_then(Value::as_u64);
        if let Some((url, ext)) = self.session_asset(&id_of(t), format).await? {
            return Ok(Media {
                source: Source::Deezer,
                title,
                artist,
                tags,
                assets: vec![Asset {
                    url,
                    ext,
                    kind: Kind::Audio,
                }],
            });
        }
        let yt_fmt = if format == Format::Mp3 {
            Format::M4a
        } else {
            format
        };
        let query = format!("{artist} {title}");
        let mut last = anyhow::anyhow!("no match");
        for cand in rank(self.yt.search(&query).await?, want) {
            match self.yt.resolve(&cand.id, true, yt_fmt).await {
                Ok(matched) => {
                    let asset = matched.assets.into_iter().next().context("no asset")?;
                    let mut tags = tags;
                    tags.push(("via".into(), "youtube".into()));
                    return Ok(Media {
                        source: Source::Deezer,
                        title,
                        artist,
                        tags,
                        assets: vec![Asset {
                            url: asset.url,
                            ext: asset.ext,
                            kind: Kind::Audio,
                        }],
                    });
                }
                Err(e) => last = e,
            }
        }
        if let Some(pv) = t.get("preview").and_then(Value::as_str) {
            return Ok(Media {
                source: Source::Deezer,
                title,
                artist,
                tags,
                assets: vec![Asset {
                    url: pv.into(),
                    ext: "mp3".into(),
                    kind: Kind::Audio,
                }],
            });
        }
        Err(last)
    }
}

pub fn kind(url: &Url) -> Result<String> {
    for seg in url.path_segments().context("no path")? {
        if matches!(seg, "track" | "album" | "playlist") {
            return Ok(seg.into());
        }
    }
    anyhow::bail!("no track/album/playlist in path")
}

fn id_after(url: &Url, kind: &str) -> Result<String> {
    let mut segs = url.path_segments().context("no path")?;
    while let Some(s) = segs.next() {
        if s == kind {
            return segs.next().context("no id").map(str::to_string);
        }
    }
    anyhow::bail!("no {kind} in path")
}

fn id_of(t: &Value) -> String {
    t.get("id")
        .and_then(Value::as_u64)
        .map(|v| v.to_string())
        .or_else(|| t.get("id").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default()
}

fn href_ext(url: &str) -> String {
    let path = url.split('?').next().unwrap_or(url);
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()) {
        Some(e)
            if !e.is_empty()
                && e.len() <= 5
                && e.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            e
        }
        _ => "mp3".into(),
    }
}

fn href_of(m: &Value) -> &str {
    m.get("HREF").and_then(Value::as_str).unwrap_or("")
}

fn pick_media(media: &[Value], format: Format) -> Option<(String, String)> {
    let full: Vec<&Value> = media
        .iter()
        .filter(|m| m.get("TYPE").and_then(Value::as_str) != Some("preview"))
        .filter(|m| m.get("HREF").and_then(Value::as_str).is_some())
        .collect();
    if full.is_empty() {
        return None;
    }
    let wanted: &[&str] = match format {
        Format::Flac => &["flac"],
        Format::Mp3 => &["mp3"],
        Format::Best => &["flac", "mp3", "aac", "m4a"],
        Format::M4a => &["m4a", "aac"],
        Format::Opus => &["opus", "ogg"],
        _ => &[],
    };
    for w in wanted {
        if let Some(m) = full.iter().find(|m| href_ext(href_of(m)) == *w) {
            return Some((href_of(m).to_string(), (*w).to_string()));
        }
    }
    let m = full[0];
    let u = href_of(m);
    Some((u.to_string(), href_ext(u)))
}

fn tags_of(t: &Value) -> Vec<(String, String)> {
    let mut tags = Vec::new();
    if let Some(d) = t.get("duration").and_then(Value::as_u64) {
        tags.push(("duration".into(), fmt_secs(d)));
    }
    if let Some(r) = t
        .get("release_date")
        .and_then(Value::as_str)
        .filter(|s| s.len() >= 4)
    {
        tags.push(("year".into(), r[..4].into()));
    }
    if let Some(b) = t.get("explicit_lyrics").and_then(Value::as_bool) {
        tags.push(("explicit".into(), b.to_string()));
    }
    if let Some(b) = t.get("bpm").and_then(Value::as_u64) {
        tags.push(("bpm".into(), b.to_string()));
    }
    if let Some(l) = t.get("label").and_then(Value::as_str) {
        tags.push(("label".into(), l.into()));
    }
    if let Some(g) = t.pointer("/genres/data").and_then(Value::as_array) {
        let names: Vec<&str> = g
            .iter()
            .filter_map(|x| x.get("name").and_then(Value::as_str))
            .collect();
        if !names.is_empty() {
            tags.push(("genres".into(), names.join(", ")));
        }
    }
    if let Some(a) = t
        .get("album")
        .and_then(|v| v.get("title"))
        .and_then(Value::as_str)
    {
        tags.push(("album".into(), a.into()));
    }
    if let Some(p) = t.get("preview").and_then(Value::as_str) {
        tags.push(("preview".into(), p.into()));
    }
    if let Some(l) = t.get("link").and_then(Value::as_str) {
        tags.push(("link".into(), l.into()));
    }
    if let Some(n) = t
        .pointer("/tracks/data")
        .and_then(Value::as_array)
        .map(Vec::len)
    {
        tags.push(("tracks".into(), n.to_string()));
    }
    tags
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(t: &str, u: &str) -> Value {
        json!({ "TYPE": t, "HREF": u })
    }

    #[test]
    fn pick_prefers_format() {
        let media = vec![
            m("preview", "https://x/p.mp3"),
            m("full", "https://x/a.flac"),
            m("full", "https://x/b.mp3?hdnea=x"),
        ];
        let (u, e) = pick_media(&media, Format::Mp3).unwrap();
        assert_eq!(e, "mp3");
        assert!(u.contains("b.mp3"));
        let (_, e) = pick_media(&media, Format::Flac).unwrap();
        assert_eq!(e, "flac");
        let (_, e) = pick_media(&media, Format::Best).unwrap();
        assert_eq!(e, "flac");
    }

    #[test]
    fn pick_none_when_only_preview() {
        let media = vec![m("preview", "https://x/p.mp3")];
        assert!(pick_media(&media, Format::Mp3).is_none());
    }

    #[test]
    fn kind_skips_locale() {
        let url = Url::parse("https://www.deezer.com/us/album/123").unwrap();
        assert_eq!(kind(&url).unwrap(), "album");
        assert_eq!(id_after(&url, "album").unwrap(), "123");
    }
}