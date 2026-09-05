//! yankee cli entry

mod adapters;
mod config;
mod download;
mod media;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use adapters::Request;

#[derive(Parser)]
#[command(name = "yankee", about = "structured downloads from ugly sources")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Info { url: String, #[arg(long)] audio: bool, #[arg(long)] proxy: Option<String> },
    Get { url: String, #[arg(long)] audio: bool, #[arg(long)] proxy: Option<String>, #[arg(long)] dir: Option<PathBuf> },
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
            print!("{}\n  source: {}\n  artist: {}\n", media.title, media.source_tag(), media.artist);
            for a in &media.assets {
                println!("  {}.{} ({:?})", a.ext, a.url, a.kind);
            }
        }
        Command::Get { url, audio, proxy, dir } => {
            if proxy.is_some() {
                cfg.proxy = proxy;
            }
            let req = with_audio(adapters::parse(&url)?, audio);
            let media = adapters::resolve(&req, &cfg).await?;
            let target = dir
                .or_else(|| cfg.dir.as_ref().map(PathBuf::from))
                .unwrap_or_else(|| PathBuf::from("downloads"));
            std::fs::create_dir_all(&target)?;
            let files = download::save(&media, &target, cfg.proxy.as_deref()).await?;
            for f in files {
                println!("{}", f.display());
            }
        }
    }
    Ok(())
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