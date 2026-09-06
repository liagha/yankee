//! spotify adapter: metadata from embed page, audio matched via youtube

use anyhow::{Context, Result};
use reqwest::Client as Http;
use serde_json::Value;
use url::Url;

use crate::media::{Asset, Format, Kind, Media, Source, fmt_secs};

use super::youtube;

const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0 Safari/537.36";

pub struct Client {
    http: Http,
    yt: youtube::Api,
}

impl Client {
    pub fn new(proxy: Option<&str>) -> Result<Self> {
        let mut builder = Http::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent(UA);
        if let Some(p) = proxy {
            builder = builder.proxy(reqwest::Proxy::all(p).context("proxy")?);
        }
        Ok(Self {
            http: builder.build().context("client")?,
            yt: youtube::Api::new(proxy)?,
        })
    }

    pub async fn track(&self, url: Url, format: Format) -> Result<Media> {
        let id = path_segment(&url, 1).context("no track id")?;
        let entity = self.entity("track", &id).await?;
        self.entity_media(&entity, format).await
    }

    pub async fn album(&self, url: Url, format: Format) -> Result<Media> {
        let id = path_segment(&url, 1).context("no album id")?;
        let entity = self.entity("album", &id).await?;
        self.entity_media(&entity, format).await
    }
}

impl Client {
    async fn entity(&self, kind: &str, id: &str) -> Result<Value> {
        let html = self
            .http
            .get(format!("https://open.spotify.com/embed/{kind}/{id}"))
            .send()
            .await?
            .text()
            .await?;
        let start = "<script id=\"__NEXT_DATA__\" type=\"application/json\">";
        let inner = html
            .find(start)
            .map(|i| i + start.len())
            .context("no embed data")?;
        let end = html[inner..].find("</script>").context("no embed end")?;
        let data: Value = serde_json::from_str(&html[inner..inner + end]).context("bad json")?;
        data.pointer("/props/pageProps/state/data/entity")
            .cloned()
            .context("no entity")
    }

    async fn entity_media(&self, entity: &Value, format: Format) -> Result<Media> {
        let title = entity
            .get("name")
            .or_else(|| entity.get("title"))
            .and_then(|v| v.as_str())
            .context("no title")?;
        let artist = entity
            .pointer("/artists/0/name")
            .or_else(|| entity.get("subtitle"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let tags = tags_of(entity);
        self.media(title.to_string(), artist, tags, format).await
    }

    async fn media(
        &self,
        title: String,
        artist: String,
        tags: Vec<(String, String)>,
        format: Format,
    ) -> Result<Media> {
        let query = format!("{artist} {title}");
        let mut last = anyhow::anyhow!("no match");
        for video_id in self.yt.search(&query).await? {
            match self.yt.resolve(&video_id, true, format).await {
                Ok(matched) => {
                    let asset = matched.assets.into_iter().next().context("no asset")?;
                    return Ok(Media {
                        source: Source::Spotify,
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
        Err(last)
    }
}

fn tags_of(entity: &Value) -> Vec<(String, String)> {
    let mut tags = Vec::new();
    if let Some(d) = entity
        .get("duration")
        .and_then(|v| v.as_u64())
        .filter(|d| *d > 0)
    {
        tags.push(("duration".into(), fmt_secs(d / 1000)));
    }
    if let Some(y) = entity
        .pointer("/releaseDate/isoString")
        .and_then(|v| v.as_str())
        .map(|s| &s[..s.len().min(4)])
        .filter(|s| s.chars().all(char::is_numeric))
    {
        tags.push(("year".into(), y.to_string()));
    }
    if let Some(b) = entity.get("isExplicit").and_then(|v| v.as_bool()) {
        tags.push(("explicit".into(), b.to_string()));
    }
    if let Some(u) = entity.pointer("/audioPreview/url").and_then(|v| v.as_str()) {
        tags.push(("preview".into(), u.to_string()));
    }
    if let Some(u) = entity.get("uri").and_then(|v| v.as_str()) {
        tags.push(("uri".into(), u.to_string()));
    }
    if let Some(n) = entity
        .pointer("/trackList")
        .and_then(|v| v.as_array())
        .map(Vec::len)
    {
        tags.push(("tracks".into(), n.to_string()));
    }
    tags
}

pub fn kind(url: &Url) -> Result<String> {
    path_segment(url, 0).context("no kind")
}

fn path_segment(url: &Url, at: usize) -> Result<String> {
    Ok(url
        .path_segments()
        .context("no path")?
        .nth(at)
        .context("segment")?
        .to_string())
}
