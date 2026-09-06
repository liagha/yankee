//! yankee cli entry

mod adapters;
mod config;
mod download;
mod media;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};
use futures::stream::{self, StreamExt};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

use adapters::Request;

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
        #[arg(long)]
        proxy: Option<String>,
    },
    Get {
        #[arg(required = true)]
        url: Vec<String>,
        #[arg(long)]
        audio: bool,
        #[arg(long)]
        proxy: Option<String>,
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
        Command::Info { url, audio, proxy } => {
            if proxy.is_some() {
                cfg.proxy = proxy;
            }
            let req = with_audio(adapters::parse(&url)?, audio);
            let media = adapters::resolve(&req, &cfg).await?;
            print!(
                "{}\n  source: {}\n  artist: {}\n",
                media.title,
                media.source_tag(),
                media.artist
            );
            for a in &media.assets {
                println!("  {}.{} ({:?})", a.ext, a.url, a.kind);
            }
            Ok(())
        }
        Command::Get {
            url,
            audio,
            proxy,
            jobs,
            dir,
        } => {
            if proxy.is_some() {
                cfg.proxy = proxy;
            }
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
                            Ok(r) => with_audio(r, audio),
                            Err(e) => return (u, Err(e)),
                        };
                        let bar = mp.add(ProgressBar::new_spinner().with_style(spinner.clone()));
                        bar.set_message(format!("resolve {u}"));
                        bar.enable_steady_tick(std::time::Duration::from_millis(80));
                        let media = adapters::resolve(&req, &cfg).await;
                        match media {
                            Ok(m) => {
                                bar.set_message(format!("{} ({})", m.title, m.source_tag()));
                                let files =
                                    download::save(&m, target, cfg.proxy.as_deref(), mp).await;
                                bar.finish();
                                (u, files)
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

fn with_audio(req: Request, audio: bool) -> Request {
    match req {
        Request::Youtube { url, .. } => Request::Youtube { url, audio },
        other => other,
    }
}

impl media::Media {
    fn source_tag(&self) -> &str {
        download::source_tag(self.source)
    }
}
