//! adapter dispatch

pub mod deezer;
pub mod instagram;
pub mod spotify;
pub mod youtube;

use anyhow::{Context, Result};
use url::Url;

use super::config::Config;
use super::media::{Format, Media};

pub enum Request {
    Youtube {
        url: String,
        audio: bool,
        format: Format,
    },
    Instagram {
        url: String,
    },
    Spotify {
        url: Url,
        kind: String,
        format: Format,
    },
    Deezer {
        url: Url,
        kind: String,
        format: Format,
    },
}

pub fn parse(input: &str) -> Result<Request> {
    let url = Url::parse(input).context("bad url")?;
    match url.host_str() {
        Some(h) if h.contains("youtube.com") || h.contains("youtu.be") => Ok(Request::Youtube {
            url: input.into(),
            audio: false,
            format: Format::Best,
        }),
        Some(h) if h.contains("instagram.com") => Ok(Request::Instagram { url: input.into() }),
        Some(h) if h.contains("open.spotify.com") => {
            let kind = spotify::kind(&url)?;
            Ok(Request::Spotify {
                url,
                kind,
                format: Format::Best,
            })
        }
        Some(h) if h.contains("deezer.com") => {
            let kind = deezer::kind(&url)?;
            Ok(Request::Deezer {
                url,
                kind,
                format: Format::Best,
            })
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

pub async fn resolve(req: &Request, config: &Config) -> Result<Vec<Media>> {
    match req {
        Request::Youtube { url, audio, format } => {
            let client = youtube::Api::new(config.proxy.as_deref())?;
            let id = youtube_id(url)?;
            Ok(vec![client.resolve(&id, *audio, *format).await?])
        }
        Request::Instagram { url } => {
            let client = instagram::Client::new(config.proxy.as_deref())?;
            Ok(vec![client.resolve(url).await?])
        }
        Request::Spotify { url, kind, format } => {
            let client = spotify::Client::new(config.proxy.as_deref())?;
            let media = match kind.as_str() {
                "track" => client.track(url.clone(), *format).await,
                "album" => client.album(url.clone(), *format).await,
                other => anyhow::bail!("unsupported kind: {other}"),
            }?;
            Ok(vec![media])
        }
        Request::Deezer { url, kind, format } => {
            let mut client =
                deezer::Client::new(config.proxy.as_deref(), config.arl.clone())?;
            match kind.as_str() {
                "track" => Ok(vec![client.track(url.clone(), *format).await?]),
                "album" => client.album(url.clone(), *format).await,
                "playlist" => client.playlist(url.clone(), *format).await,
                other => anyhow::bail!("unsupported kind: {other}"),
            }
        }
    }
}
