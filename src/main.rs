//! yankee cli entry

mod adapters;
mod config;
mod download;
mod media;
mod transcode;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};
use futures::stream::{self, StreamExt};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

use adapters::Request;
use media::Format;

#[derive(Parser)]
#[command(name = "yankee", about = "structured downloads from ugly sources")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Info {
        url: String,
        #[arg(long)]
        audio: bool,
        #[arg(long, default_value = "best")]
        format: Format,
        #[arg(long)]
        proxy: Option<String>,
        #[arg(long)]
        arl: Option<String>,
    },
    Get {
        #[arg(required = true)]
        url: Vec<String>,
        #[arg(long)]
        audio: bool,
        #[arg(long, default_value = "best")]
        format: Format,
        #[arg(long)]
        proxy: Option<String>,
        #[arg(long)]
        arl: Option<String>,
        #[arg(long, default_value_t = 4)]
        jobs: usize,
        #[arg(long)]
        dir: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut cfg = config::Config::load();
    match cli.command {
        Command::Info {
            url,
            audio,
            format,
            proxy,
            arl,
        } => {
            if proxy.is_some() {
                cfg.proxy = proxy;
            }
            apply_arl(&mut cfg, arl);
            let req = with_opts(adapters::parse(&url)?, audio, format);
            let media = adapters::resolve(&req, &cfg).await?;
            for (i, m) in media.iter().enumerate() {
                if i > 0 {
                    println!();
                }
                print!(
                    "{}\n  source: {}\n  artist: {}\n",
                    m.title,
                    m.source_tag(),
                    m.artist
                );
                for (k, v) in &m.tags {
                    println!("  {k}: {v}");
                }
                for a in &m.assets {
                    println!("  {}.{} ({:?})", a.ext, a.url, a.kind);
                }
            }
            Ok(())
        }
        Command::Get {
            url,
            audio,
            format,
            proxy,
            arl,
            jobs,
            dir,
        } => {
            if proxy.is_some() {
                cfg.proxy = proxy;
            }
            apply_arl(&mut cfg, arl);
            let target = dir
                .or_else(|| cfg.dir.as_ref().map(PathBuf::from))
                .unwrap_or_else(|| PathBuf::from("downloads"));
            std::fs::create_dir_all(&target)?;
            let cfg = Arc::new(cfg);
            let mp = MultiProgress::new();
            let spinner =
                ProgressStyle::with_template("{spinner:.cyan} {msg}").expect("spinner template");
            let results: Vec<(&String, Result<Vec<PathBuf>>)> = stream::iter(url.iter())
                .map(|u| {
                    let mp = &mp;
                    let target = &target;
                    let spinner = &spinner;
                    let cfg = cfg.clone();
                    async move {
                        let req = match adapters::parse(u) {
                            Ok(r) => with_opts(r, audio, format),
                            Err(e) => return (u, Err(e)),
                        };
                        let bar = mp.add(ProgressBar::new_spinner().with_style(spinner.clone()));
                        bar.set_message(format!("resolve {u}"));
                        bar.enable_steady_tick(std::time::Duration::from_millis(80));
                        let media = adapters::resolve(&req, &cfg).await;
                        match media {
                            Ok(list) => {
                                let mut files = Vec::new();
                                for m in &list {
                                    bar.set_message(format!("{} ({})", m.title, m.source_tag()));
                                    match download::save(
                                        m,
                                        target,
                                        cfg.proxy.as_deref(),
                                        mp,
                                        format,
                                    )
                                    .await
                                    {
                                        Ok(f) => files.extend(f),
                                        Err(e) => {
                                            bar.abandon_with_message(format!("failed {u}"));
                                            return (u, Err(e));
                                        }
                                    }
                                }
                                bar.finish();
                                (u, Ok(files))
                            }
                            Err(e) => {
                                bar.abandon_with_message(format!("failed {u}"));
                                (u, Err(e))
                            }
                        }
                    }
                })
                .buffer_unordered(jobs)
                .collect()
                .await;
            mp.clear().ok();
            let n = results.len();
            let mut failed = Vec::new();
            for (u, r) in results {
                match r {
                    Ok(files) => {
                        for f in files {
                            println!("{}", f.display());
                        }
                    }
                    Err(e) => failed.push((u.clone(), e)),
                }
            }
            for (u, e) in &failed {
                eprintln!("!! {u}: {e:#}");
            }
            if failed.is_empty() {
                Ok(())
            } else {
                Err(anyhow::anyhow!("{} of {n} failed", failed.len()))
            }
        }
    }
}

fn with_opts(req: Request, audio: bool, format: Format) -> Request {
    match req {
        Request::Youtube { url, .. } => Request::Youtube { url, audio, format },
        Request::Spotify { url, kind, .. } => Request::Spotify { url, kind, format },
        Request::Deezer { url, kind, .. } => Request::Deezer { url, kind, format },
        other => other,
    }
}

fn apply_arl(cfg: &mut config::Config, arl: Option<String>) {
    cfg.arl = arl
        .or_else(|| std::env::var("DEEZER_ARL").ok())
        .or(cfg.arl.clone());
}

impl media::Media {
    fn source_tag(&self) -> &str {
        download::source_tag(self.source)
    }
}
