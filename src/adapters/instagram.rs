//! instagram adapter: self-scrape embed page contextJSON

use anyhow::{Context, Result};
use serde_json::Value;

use crate::media::{Asset, Kind, Media, Source};

const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

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
        let url = shortcode_url(input)?;
        let html = self.http.get(url).send().await?.text().await?;
        let raw = context_json(&html).context("media")?;
        parse(&raw)
    }
}

fn shortcode_url(input: &str) -> Result<String> {
    let url = url::Url::parse(input).context("bad url")?;
    let segs: Vec<String> = url
        .path_segments()
        .context("no path")?
        .map(|s| s.to_string())
        .collect();
    let code = segs.get(1).context("no shortcode")?;
    Ok(format!(
        "https://www.instagram.com/p/{code}/embed/captioned/"
    ))
}

fn context_json(html: &str) -> Result<String> {
    let head = "\"contextJSON\":\"";
    let Some(start) = html.find(head) else {
        anyhow::bail!("no media (login or bot check)")
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

fn parse(raw: &str) -> Result<Media> {
    let root: Value = serde_json::from_str(raw).context("json")?;
    let node = root
        .pointer("/gql_data/shortcode_media")
        .with_context(|| "no node in media")?;

    let title = node
        .pointer("/edge_media_to_caption/edges/0/node/text")
        .or_else(|| node.pointer("/caption/text"))
        .and_then(|v| v.as_str())
        .map(|s| s.lines().next().unwrap_or("").to_string())
        .unwrap_or_default();
    let artist = node
        .pointer("/owner/username")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    let mut assets = Vec::new();
    if let Some(edges) = node
        .pointer("/edge_sidecar_to_children/edges")
        .and_then(|e| e.as_array())
    {
        let count = node
            .pointer("/edge_sidecar_to_children/count")
            .and_then(|v| v.as_u64())
            .unwrap_or(edges.len() as u64) as usize;
        for edge in edges.iter().take(count) {
            if let Some(a) = asset_of(&edge["node"]) {
                assets.push(a);
            }
        }
    } else if let Some(a) = asset_of(node) {
        assets.push(a);
    }
    if assets.is_empty() {
        anyhow::bail!("no media")
    }
    Ok(Media {
        source: Source::Instagram,
        title,
        artist,
        assets,
    })
}

fn asset_of(node: &Value) -> Option<Asset> {
    if let Some(u) = node.get("video_url").and_then(|v| v.as_str()) {
        return Some(Asset {
            url: u.to_string(),
            ext: "mp4".into(),
            kind: Kind::Video,
        });
    }
    node.get("display_url")
        .and_then(|v| v.as_str())
        .map(|u| Asset {
            url: u.to_string(),
            ext: "jpg".into(),
            kind: Kind::Image,
        })
}
