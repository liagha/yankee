//! instagram adapter: anonymous media via graphql, embed, or og-meta

use anyhow::{Context, Result};
use serde_json::Value;

use crate::media::{Asset, Kind, Media, Source};

const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const APP_ID: &str = "936619743392459";

pub struct Client {
    http: reqwest::Client,
}

impl Client {
    pub fn new(proxy: Option<&str>) -> Result<Self> {
        let mut builder = reqwest::Client::builder().user_agent(UA);
        if let Some(p) = proxy {
            builder = builder.proxy(reqwest::Proxy::all(p).context("proxy")?);
        }
        Ok(Self {
            http: builder.build().context("client")?,
        })
    }

    pub async fn resolve(&self, input: &str) -> Result<Media> {
        let code = shortcode(input)?;
        if let Ok(m) = self.api(&code).await {
            return Ok(m);
        }
        if let Ok(m) = self.embed(&code).await {
            return Ok(m);
        }
        self.meta(&code)
            .await
            .context("no public media (login or bot check)")
    }

    async fn api(&self, code: &str) -> Result<Media> {
        let body = self
            .http
            .get(format!("https://www.instagram.com/p/{code}/?__a=1"))
            .header("x-ig-app-id", APP_ID)
            .header("x-ig-www-claim", "0")
            .header("Accept", "application/json")
            .send()
            .await?
            .text()
            .await?;
        let body = body
            .strip_prefix("for (;;);")
            .or_else(|| body.strip_prefix("while(1);"))
            .unwrap_or(&body);
        let root: Value = serde_json::from_str(body).context("bad json")?;
        if root.get("error").is_some() {
            anyhow::bail!("blocked")
        }
        let node = root
            .pointer("/data/xdt_shortcode_media")
            .or_else(|| root.pointer("/graphql/shortcode_media"))
            .with_context(|| "no graphql node")?;
        parse(node)
    }

    async fn embed(&self, code: &str) -> Result<Media> {
        let html = self
            .http
            .get(format!(
                "https://www.instagram.com/p/{code}/embed/captioned/"
            ))
            .send()
            .await?
            .text()
            .await?;
        let raw = context_json(&html)?;
        let root: Value = serde_json::from_str(&raw).context("json")?;
        let node = root
            .pointer("/gql_data/shortcode_media")
            .with_context(|| "no embed node")?;
        parse(node)
    }

    async fn meta(&self, code: &str) -> Result<Media> {
        let html = self
            .http
            .get(format!("https://www.instagram.com/p/{code}/"))
            .send()
            .await?
            .text()
            .await?;
        let mut assets = Vec::new();
        if let Some(url) = og(&html, "video") {
            assets.push(asset(url, "mp4", Kind::Video));
        }
        if let Some(url) = og(&html, "image") {
            assets.push(asset(url, "jpg", Kind::Image));
        }
        if assets.is_empty() {
            anyhow::bail!("no og media")
        }
        let title = og(&html, "title").unwrap_or_default();
        Ok(Media {
            source: Source::Instagram,
            title,
            artist: String::new(),
            tags: Vec::new(),
            assets,
        })
    }
}

fn shortcode(input: &str) -> Result<String> {
    let url = url::Url::parse(input).context("bad url")?;
    url.path_segments()
        .context("no path")?
        .nth(1)
        .map(str::to_string)
        .context("no shortcode")
}

fn context_json(html: &str) -> Result<String> {
    let head = "\"contextJSON\":\"";
    let Some(start) = html.find(head) else {
        anyhow::bail!("no embed data")
    };
    let body = &html[start + head.len()..];
    let mut end = 0;
    loop {
        let Some(rel) = body[end..].find('"') else {
            anyhow::bail!("unterminated")
        };
        let abs = end + rel;
        if is_escaped(body, abs) {
            end = abs + 1;
            continue;
        }
        let lit = format!("\"{}\"", &body[..abs]);
        return serde_json::from_str::<String>(&lit).context("decode");
    }
}

fn is_escaped(body: &str, at: usize) -> bool {
    let mut backslashes = 0;
    for b in body[..at].bytes().rev() {
        if b == b'\\' {
            backslashes += 1;
        } else {
            break;
        }
    }
    backslashes % 2 == 1
}

fn og(html: &str, prop: &str) -> Option<String> {
    let pats = [
        format!("property=\"og:{prop}:secure_url\" content=\""),
        format!("property=\"og:{prop}\" content=\""),
    ];
    for pat in pats {
        if let Some(i) = html.find(&pat) {
            let rest = &html[i + pat.len()..];
            let end = rest.find('"')?;
            if !rest[..end].is_empty() {
                return Some(rest[..end].to_string());
            }
        }
    }
    None
}

fn parse(node: &Value) -> Result<Media> {
    let title = caption(node).unwrap_or_default();
    let artist = node
        .pointer("/owner/username")
        .or_else(|| node.pointer("/user/username"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let assets = assets_of(node);
    if assets.is_empty() {
        anyhow::bail!("no media in node")
    }
    Ok(Media {
        source: Source::Instagram,
        title,
        artist,
        tags: Vec::new(),
        assets,
    })
}

fn caption(node: &Value) -> Option<String> {
    node.pointer("/edge_media_to_caption/edges/0/node/text")
        .or_else(|| node.pointer("/xdt_media_to_caption/edges/0/node/text"))
        .or_else(|| node.pointer("/caption/text"))
        .and_then(|v| v.as_str())
        .map(|s| s.lines().next().unwrap_or("").to_string())
}

fn assets_of(node: &Value) -> Vec<Asset> {
    let mut assets = Vec::new();
    if let Some(edges) = node
        .pointer("/edge_sidecar_to_children/edges")
        .or_else(|| node.pointer("/xdt_sidecar_to_children/edges"))
        .and_then(|e| e.as_array())
    {
        for edge in edges {
            if let Some(a) = asset_of(&edge["node"]) {
                assets.push(a);
            }
        }
    } else if let Some(a) = asset_of(node) {
        assets.push(a);
    }
    assets
}

fn asset_of(node: &Value) -> Option<Asset> {
    if let Some(u) = node
        .get("video_url")
        .or_else(|| node.pointer("/video_versions/0/url"))
        .and_then(|v| v.as_str())
    {
        return Some(asset(u.to_string(), "mp4", Kind::Video));
    }
    node.get("display_url")
        .and_then(|v| v.as_str())
        .map(|u| asset(u.to_string(), "jpg", Kind::Image))
}

fn asset(url: String, ext: &str, kind: Kind) -> Asset {
    Asset {
        url,
        ext: ext.to_string(),
        kind,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIDECAR_OLD: &str = r#""edge_sidecar_to_children":{"count":2,"edges":[{"node":{"display_url":"https://cd/a.jpg"}},{"node":{"video_url":"https://cd/b.mp4"}}]}"#;
    const SIDECAR_NEW: &str = r#""xdt_sidecar_to_children":{"count":2,"edges":[{"node":{"display_url":"https://cd/a.jpg"}},{"node":{"video_url":"https://cd/b.mp4"}}]}"#;

    fn node(caption_key: &str, owner_key: &str, sidecar: &str) -> Value {
        let json = format!(
            r#"{{"{caption_key}":{{"edges":[{{"node":{{"text":"hello world\nline two"}}}}]}},"{owner_key}":{{"username":"some_artist"}},"display_url":"https://cd/1.jpg","video_url":"https://cd/1.mp4",{sidecar}}}"#
        );
        serde_json::from_str(&json).unwrap()
    }

    #[test]
    fn parses_old_schema() {
        let m = parse(&node("edge_media_to_caption", "owner", SIDECAR_OLD)).unwrap();
        assert_eq!(m.artist, "some_artist");
        assert_eq!(m.title, "hello world");
        assert_eq!(m.assets.len(), 2);
    }

    #[test]
    fn parses_new_schema() {
        let m = parse(&node("xdt_media_to_caption", "user", SIDECAR_NEW)).unwrap();
        assert_eq!(m.artist, "some_artist");
        assert_eq!(m.assets.len(), 2);
    }

    #[test]
    fn parses_single_node() {
        let n = serde_json::json!({
            "edge_media_to_caption": {"edges": [{"node": {"text": "solo"}}]},
            "owner": {"username": "u"},
            "video_url": "https://cd/v.mp4",
            "video_versions": [{"url": "https://cd/v.mp4"}],
            "display_url": "https://cd/v.jpg"
        });
        let m = parse(&n).unwrap();
        assert_eq!(m.assets.len(), 1);
        assert_eq!(m.assets[0].kind, Kind::Video);
        assert_eq!(m.assets[0].ext, "mp4");
    }

    #[test]
    fn context_json_handles_escapes() {
        let html = r#"data="contextJSON":"{\"a\":\"x\"}""#;
        let raw = context_json(html).unwrap();
        assert_eq!(raw.as_str(), r#"{"a":"x"}"#);
    }

    #[test]
    fn extracts_og_tags() {
        let html = r##"<meta property="og:video:secure_url" content="https://cd/v.mp4"><meta property="og:image" content="https://cd/i.jpg">"##;
        assert_eq!(og(html, "video").unwrap(), "https://cd/v.mp4");
        assert_eq!(og(html, "image").unwrap(), "https://cd/i.jpg");
    }
}
