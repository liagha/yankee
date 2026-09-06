//! shared downloader writing assets to disk

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use futures::StreamExt;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use tokio::io::AsyncWriteExt;

use crate::media::{Format, Kind, Media, Source};
use crate::transcode;

pub async fn save(
    media: &Media,
    dir: &Path,
    proxy: Option<&str>,
    mp: &MultiProgress,
    format: Format,
) -> Result<Vec<PathBuf>> {
    let http = client(proxy)?;
    let mut out = Vec::new();
    for (asset, name) in media.assets.iter().zip(shared_names(media, dir)) {
        let (final_name, tmp) = if format.transcode() && asset.kind == Kind::Audio {
            let ext = format.label();
            (name.with_extension(ext), name.with_extension("part"))
        } else {
            (name.clone(), name)
        };
        let res = http
            .get(&asset.url)
            .header("Range", "bytes=0-")
            .send()
            .await?;
        let total = res.content_length().unwrap_or(0);
        let bar = mp.add(ProgressBar::new(total));
        bar.set_style(if total > 0 {
            bar_style()
        } else {
            count_style()
        });
        bar.set_message(
            final_name
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
        );
        let stream = res.bytes_stream();
        let mut file = tokio::fs::File::create(&tmp)
            .await
            .with_context(|| format!("create {tmp:?}"))?;
        futures::pin_mut!(stream);
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            file.write_all(&chunk).await?;
            bar.inc(chunk.len() as u64);
        }
        file.flush().await?;
        bar.finish_and_clear();
        if tmp != final_name {
            let src = tmp.clone();
            let dst = final_name.clone();
            tokio::task::spawn_blocking(move || match format {
                Format::Flac => transcode::to_flac(&src, &dst),
                Format::Wav => transcode::to_wav(&src, &dst),
                _ => Ok(()),
            })
            .await
            .context("transcode task")??;
            tokio::fs::remove_file(tmp).await?;
        }
        out.push(final_name);
    }
    Ok(out)
}

fn client(proxy: Option<&str>) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(300));
    if let Some(p) = proxy {
        builder = builder.proxy(reqwest::Proxy::all(p).context("proxy")?);
    }
    builder.build().context("client")
}

fn bar_style() -> ProgressStyle {
    ProgressStyle::with_template(
        "{bar:32.green/black} {percent:>3}% {bytes}/{total_bytes} [{binary_bytes_per_sec}] {msg}",
    )
    .expect("bar template")
    .progress_chars("▓░")
}

fn count_style() -> ProgressStyle {
    ProgressStyle::with_template("{spinner:.green} {bytes} {msg}").expect("count template")
}

fn shared_names(media: &Media, dir: &Path) -> Vec<PathBuf> {
    let stem = stem(&media.title);
    let mut used = std::collections::HashSet::new();
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
    if s.is_empty() { "download".into() } else { s }
}
