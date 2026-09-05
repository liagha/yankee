//! adapter dispatch

pub mod instagram;
pub mod spotify;
pub mod youtube;

use anyhow::{Context, Result};
use url::Url;

use super::config::Config;
use super::media::Media;

pub enum Request {
    Youtube { url: String, audio: bool },
    Instagram { url: String },
    Spotify { url: Url, kind: String },
}

pub fn parse(input: &str) -> Result<Request> {
    let url = Url::parse(input).context("bad url")?;
    match url.host_str() {
        Some(h) if h.contains("youtube.com") || h.contains("youtu.be") => {
            Ok(Request::Youtube { url: input.into(), audio: false })
        }
        Some(h) if h.contains("instagram.com") => Ok(Request::Instagram { url: input.into() }),
        Some(h) if h.contains("open.spotify.com") => {
            let kind = spotify::kind(&url)?;
            Ok(Request::Spotify { url, kind })
        }
        _ => anyhow::bail!("unsupported host"),
    }
}

fn youtube_id(input: &str) -> Result<String> {
    let url = Url::parse(input).context("bad url")?;
    if url.host_str() == Some("youtu.be") {
        return url
            .path_segments()
            .context("no path")?
            .next()
            .map(|s| s.to_string())
            .context("no id");
    }
    let id = url
        .query_pairs()
        .find(|(k, _)| k == "v")
        .map(|(_, v)| v.to_string());
    if id.as_deref().map(str::is_empty) == Some(true) {
        anyhow::bail!("empty id")
    }
    id.context("no id")
}

pub async fn resolve(req: &Request, config: &Config) -> Result<Media> {
    match req {
        Request::Youtube { url, audio } => {
            let client = youtube::Api::new(config.proxy.as_deref())?;
            let id = youtube_id(url)?;
            Ok(client.resolve(&id, *audio).await?)
        }
        Request::Instagram { url } => {
            let client = instagram::Client::new(config.proxy.as_deref())?;
            client.resolve(url).await
        }
        Request::Spotify { url, kind } => {
            let client = spotify::Client::new(config.proxy.as_deref())?;
            match kind.as_str() {
                "track" => client.track(url.clone()).await,
                "album" => client.album(url.clone()).await,
                other => anyhow::bail!("unsupported kind: {other}"),
            }
        }
    }
}