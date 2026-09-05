//! shared downloader writing assets to disk

use anyhow::{Context, Result};

use crate::media::{Media, Source};

pub async fn save(media: &Media, dir: &std::path::Path, proxy: Option<&str>) -> Result<Vec<std::path::PathBuf>> {
    let http = client(proxy)?;
    let mut out = Vec::new();
    for (asset, candidate) in media.assets.iter().zip(shared_names(media, dir)) {
        let res = http.get(&asset.url).header("Range", "bytes=0-").send().await?;
        let bytes = res.error_for_status()?.bytes().await?;
        std::fs::write(&candidate, bytes).with_context(|| format!("write {candidate:?}"))?;
        out.push(candidate);
    }
    Ok(out)
}

fn client(proxy: Option<&str>) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder().timeout(std::time::Duration::from_secs(300));
    if let Some(p) = proxy {
        builder = builder.proxy(reqwest::Proxy::all(p).context("proxy")?);
    }
    builder.build().context("client")
}

fn shared_names(media: &Media, dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let stem = stem(&media.title);
    let mut used: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();
    media
        .assets
        .iter()
        .enumerate()
        .map(|(i, asset)| {
            let mut name = stem.clone();
            loop {
                let candidate = dir.join(format!("{name}.{}", asset.ext));
                if used.insert(candidate.clone()) {
                    return candidate;
                }
                name = format!("{stem}-{i}");
            }
        })
        .collect()
}

pub fn source_tag(source: Source) -> &'static str {
    match source {
        Source::Youtube => "youtube",
        Source::Instagram => "instagram",
        Source::Spotify => "spotify",
    }
}

fn stem(title: &str) -> String {
    let mut s = String::new();
    for c in title.chars() {
        match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | ' ' => s.push(c),
            '.' | ',' | ':' | '/' | '\\' | '(' | ')' | '[' | ']' | '{' | '}' => s.push(' '),
            _ if c.is_alphanumeric() => s.push(c),
            _ => s.push(' '),
        }
    }
    let s = s.split_whitespace().collect::<Vec<_>>().join("-");
    if s.is_empty() {
        "download".into()
    } else {
        s
    }
}