//! spotify adapter: metadata from embed page, audio matched via youtube

use anyhow::{Context, Result};
use reqwest::Client as Http;
use serde_json::Value;
use url::Url;

use crate::media::{Asset, Kind, Media, Source};

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

    pub async fn track(&self, url: Url) -> Result<Media> {
        let id = path_segment(&url, 1).context("no track id")?;
        let entity = self.entity("track", &id).await?;
        let title = entity
            .opt("name")
            .or(entity.opt("title"))
            .context("no title")?;
        let artist = entity
            .pointer("/artists/0/name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        self.media(title, artist).await
    }

    pub async fn album(&self, url: Url) -> Result<Media> {
        let id = path_segment(&url, 1).context("no album id")?;
        let entity = self.entity("album", &id).await?;
        let title = entity
            .opt("name")
            .or(entity.opt("title"))
            .context("no title")?;
        let artist = entity
            .pointer("/artists/0/name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        self.media(title, artist).await
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

    async fn media(&self, title: String, artist: String) -> Result<Media> {
        let query = format!("{artist} {title}");
        let mut last = anyhow::anyhow!("no match");
        for video_id in self.yt.search(&query).await? {
            match self.yt.resolve(&video_id, true).await {
                Ok(matched) => {
                    let asset = matched.assets.into_iter().next().context("no asset")?;
                    return Ok(Media {
                        source: Source::Spotify,
                        title,
                        artist,
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

trait Opt {
    fn opt(&self, key: &str) -> Option<String>;
}

impl Opt for Value {
    fn opt(&self, key: &str) -> Option<String> {
        self.get(key).and_then(|v| v.as_str()).map(str::to_string)
    }
}
