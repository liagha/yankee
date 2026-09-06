//! youtube adapter over innertube api

use anyhow::{Context, Result};
use reqwest::{Client, header::HeaderMap};
use serde_json::Value;

use crate::media::{Asset, Format, Kind, Media, Source, fmt_secs};

const KEY: &str = "AIzaSyAO_FJ2SlqU8Q4STEHLGCilw_Y9_11qcW8";

#[derive(Clone)]
struct Spec {
    name: &'static str,
    version: &'static str,
    num: u32,
    make: &'static str,
    model: &'static str,
    agent: &'static str,
    os: &'static str,
    os_ver: &'static str,
}

const CLIENTS: &[Spec] = &[
    // unthrottled plaintext urls
    Spec {
        name: "VISIONOS",
        version: "1.02",
        num: 101,
        make: "Apple",
        model: "RealityDevice17,1",
        agent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15",
        os: "visionOS",
        os_ver: "26.5.23O471",
    },
    // fallback, throttled on label-owned content
    Spec {
        name: "IOS",
        version: "20.41.2",
        num: 5,
        make: "Apple",
        model: "iPhone16,2",
        agent: "com.google.ios.youtube/20.41.2 (iPhone16,2; U; CPU iOS 18_3_2 like Mac OS X;)",
        os: "iPhone",
        os_ver: "18.3.2.22D82",
    },
    Spec {
        name: "ANDROID_VR",
        version: "1.51.16.2524",
        num: 28,
        make: "Oculus",
        model: "Quest 3",
        agent: "com.google.android.apps.youtube.vr.oculus/1.51.16.2524 (Linux; U; Android 12; eureka 1.51.16.2524) gzip",
        os: "Android",
        os_ver: "12",
    },
];

pub struct Api {
    http: Client,
    visitor: std::sync::RwLock<Option<String>>,
}

impl Api {
    pub fn new(proxy: Option<&str>) -> Result<Self> {
        let mut builder = Client::builder().timeout(std::time::Duration::from_secs(30));
        if let Some(p) = proxy {
            builder = builder.proxy(reqwest::Proxy::all(p).context("proxy")?);
        }
        Ok(Self {
            http: builder.build().context("client")?,
            visitor: std::sync::RwLock::new(None),
        })
    }

    pub async fn resolve(&self, video_id: &str, audio: bool, format: Format) -> Result<Media> {
        let mut last = anyhow::anyhow!("no stream");
        for spec in CLIENTS {
            match self.player(video_id, spec).await {
                Ok(player) => return self.pick(&player, audio, format),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    pub async fn search(&self, query: &str) -> Result<Vec<String>> {
        let mut last = anyhow::anyhow!("no results");
        for spec in CLIENTS {
            match self.query(query, spec).await {
                Ok(ids) if !ids.is_empty() => return Ok(ids),
                Ok(_) => last = anyhow::anyhow!("no results"),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    async fn api(&self, path: &str, body: serde_json::Value, spec: &Spec) -> Result<Value> {
        let mut hdrs = HeaderMap::new();
        hdrs.insert(
            "X-YouTube-Client-Name",
            spec.num.to_string().parse().unwrap(),
        );
        hdrs.insert("X-YouTube-Client-Version", spec.version.parse().unwrap());
        let mut body = body;
        body["context"]["client"]["deviceMake"] = spec.make.into();
        body["context"]["client"]["deviceModel"] = spec.model.into();
        body["context"]["client"]["userAgent"] = spec.agent.into();
        body["context"]["client"]["osName"] = spec.os.into();
        body["context"]["client"]["osVersion"] = spec.os_ver.into();
        if let Some(v) = self.visitor.read().unwrap().as_ref() {
            body["context"]["client"]["visitorData"] = v.as_str().into();
        }
        let req = self
            .http
            .post(format!(
                "https://www.youtube.com/youtubei/v1/{path}?key={KEY}"
            ))
            .headers(hdrs)
            .json(&body);
        let data: Value = req.send().await?.json().await?;
        self.store_visitor(&data);
        Ok(data)
    }

    fn store_visitor(&self, data: &Value) {
        if let Some(v) = data
            .pointer("/responseContext/visitorData")
            .and_then(|x| x.as_str())
        {
            let mut vd = self.visitor.write().unwrap();
            if vd.as_deref() != Some(v) {
                *vd = Some(v.to_string());
            }
        }
    }

    fn forget_visitor(&self) {
        *self.visitor.write().unwrap() = None;
    }

    async fn player(&self, video_id: &str, spec: &Spec) -> Result<Value> {
        let body = |video_id: &str| {
            serde_json::json!({
                "context": {"client": {"clientName": spec.name, "clientVersion": spec.version}},
                "videoId": video_id,
            })
        };
        let player = self.api("player", body(video_id), spec).await?;
        let gated = status(&player) != "OK";
        if gated {
            let retry = self.api("player", body(video_id), spec).await?;
            if status(&retry) == "OK" {
                return Ok(retry);
            }
            self.forget_visitor();
            let st = status(&player);
            let reason = player
                .pointer("/playabilityStatus/reason")
                .and_then(|v| v.as_str())
                .unwrap_or(&st)
                .to_string();
            anyhow::bail!("{reason}");
        }
        Ok(player)
    }

    async fn query(&self, query: &str, spec: &Spec) -> Result<Vec<String>> {
        let body = serde_json::json!({
            "context": {"client": {"clientName": spec.name, "clientVersion": spec.version}},
            "query": query,
        });
        let data = self.api("search", body, spec).await?;
        Ok(collect(data))
    }

    fn pick(&self, player: &Value, audio: bool, format: Format) -> Result<Media> {
        let title = player
            .pointer("/videoDetails/title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let artist = player
            .pointer("/videoDetails/author")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let formats = player
            .pointer("/streamingData/adaptiveFormats")
            .and_then(|v| v.as_array())
            .context("no formats")?;

        let audio = audio || format.is_audio();
        let mut best = Best::new();
        for f in formats {
            let Some(url) = f.get("url").and_then(|v| v.as_str()) else {
                continue;
            };
            let essence = f
                .get("mimeType")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .split(';')
                .next()
                .unwrap_or("");
            if !fits(essence, audio, format) {
                continue;
            }
            let score = if audio {
                f.get("bitrate").and_then(|v| v.as_u64()).unwrap_or(0)
            } else {
                f.get("height").and_then(|v| v.as_u64()).unwrap_or(0)
            };
            best.update(
                url,
                crate::media::ext_of(essence),
                if audio { Kind::Audio } else { Kind::Video },
                score,
            );
        }

        let asset = best
            .take()
            .context(format!("no {} stream", format.label()))?;
        Ok(Media {
            source: Source::Youtube,
            title,
            artist,
            tags: tags_of(player),
            assets: vec![asset],
        })
    }
}

pub fn fits(essence: &str, audio: bool, format: Format) -> bool {
    if format.is_audio() {
        let want = match format {
            Format::M4a => "audio/mp4",
            _ => "audio/webm",
        };
        return essence == want;
    }
    let is_video = essence.starts_with("video");
    let is_audio = essence.starts_with("audio");
    match format {
        Format::Mp4 => essence == if audio { "audio/mp4" } else { "video/mp4" },
        Format::Webm => essence == if audio { "audio/webm" } else { "video/webm" },
        Format::Best if audio => is_audio,
        _ => is_video,
    }
}

fn str_of(player: &Value, at: &str) -> Option<String> {
    let v = player.pointer(at)?;
    if let Some(s) = v.as_str() {
        Some(s.to_string())
    } else {
        v.as_u64().map(|n| n.to_string())
    }
}

fn tags_of(player: &Value) -> Vec<(String, String)> {
    let mut tags = Vec::new();
    if let Some(s) = str_of(player, "/videoDetails/lengthSeconds").and_then(|s| s.parse().ok()) {
        tags.push(("duration".into(), fmt_secs(s)));
    }
    if let Some(s) = str_of(player, "/videoDetails/viewCount") {
        tags.push(("views".into(), s));
    }
    if let Some(u) = player
        .pointer("/videoDetails/thumbnail/thumbnails")
        .and_then(|v| v.as_array())
        .and_then(|a| {
            a.iter()
                .max_by_key(|t| t.get("width").and_then(|w| w.as_u64()).unwrap_or(0))
        })
        .and_then(|t| t.get("url"))
        .and_then(|v| v.as_str())
    {
        tags.push(("cover".into(), u.to_string()));
    }
    tags
}

fn status(player: &Value) -> String {
    player
        .pointer("/playabilityStatus/status")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn collect(v: Value) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    collect_in(&v, &mut out, &mut seen);
    out
}

fn collect_in(v: &Value, out: &mut Vec<String>, seen: &mut std::collections::HashSet<String>) {
    match v {
        Value::Object(map) => {
            if let Some(v) = map.get("videoId").and_then(|x| x.as_str()) {
                if seen.insert(v.to_string()) {
                    out.push(v.to_string());
                }
                return;
            }
            for x in map.values() {
                collect_in(x, out, seen);
            }
        }
        Value::Array(list) => {
            for x in list {
                collect_in(x, out, seen);
            }
        }
        _ => {}
    }
}

struct Best {
    url: String,
    ext: String,
    kind: Kind,
    score: u64,
    set: bool,
}

impl Best {
    fn new() -> Self {
        Self {
            url: String::new(),
            ext: String::new(),
            kind: Kind::Video,
            score: 0,
            set: false,
        }
    }

    fn update(&mut self, url: &str, ext: String, kind: Kind, score: u64) {
        if !self.set || score > self.score {
            self.url = url.to_string();
            self.ext = ext;
            self.kind = kind;
            self.score = score;
            self.set = true;
        }
    }

    fn take(&mut self) -> Option<Asset> {
        if self.set {
            self.set = false;
            Some(Asset {
                url: self.url.clone(),
                ext: self.ext.clone(),
                kind: self.kind,
            })
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_best() {
        assert!(fits("audio/webm; codecs=opus", true, Format::Best));
        assert!(fits("audio/mp4; codecs=mp4a", true, Format::Best));
        assert!(!fits("video/webm", true, Format::Best));
    }

    #[test]
    fn video_best() {
        assert!(fits("video/mp4", false, Format::Best));
        assert!(!fits("audio/mp4", false, Format::Best));
    }

    #[test]
    fn audio_containers() {
        assert!(fits("audio/mp4", true, Format::Mp4));
        assert!(!fits("audio/webm", true, Format::Mp4));
        assert!(fits("audio/webm", true, Format::Webm));
        assert!(!fits("audio/mp4", true, Format::Webm));
    }

    #[test]
    fn video_containers() {
        assert!(fits("video/mp4", false, Format::Mp4));
        assert!(!fits("video/webm", false, Format::Mp4));
        assert!(fits("video/webm", false, Format::Webm));
        assert!(!fits("video/mp4", false, Format::Webm));
    }

    #[test]
    fn audio_only() {
        assert!(fits("audio/webm", false, Format::Opus));
        assert!(!fits("video/webm", false, Format::Opus));
        assert!(fits("audio/mp4", false, Format::M4a));
        assert!(!fits("video/mp4", false, Format::M4a));
    }
}
