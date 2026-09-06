//! shared model every adapter returns

use clap::ValueEnum;
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub enum Source {
    Youtube,
    Instagram,
    Spotify,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub enum Kind {
    Video,
    Audio,
    Image,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, ValueEnum)]
pub enum Format {
    #[default]
    Best,
    Mp4,
    Webm,
    M4a,
    Opus,
}

impl Format {
    pub fn label(self) -> &'static str {
        match self {
            Format::Best => "best",
            Format::Mp4 => "mp4",
            Format::Webm => "webm",
            Format::M4a => "m4a",
            Format::Opus => "opus",
        }
    }

    pub fn is_audio(self) -> bool {
        matches!(self, Format::M4a | Format::Opus)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Asset {
    pub url: String,
    pub ext: String,
    pub kind: Kind,
}

#[derive(Clone, Debug, Serialize)]
pub struct Media {
    pub source: Source,
    pub title: String,
    pub artist: String,
    pub tags: Vec<(String, String)>,
    pub assets: Vec<Asset>,
}

pub fn fmt_secs(s: u64) -> String {
    format!("{}:{:02}", s / 60, s % 60)
}

pub fn ext_of(essence: &str) -> String {
    match essence {
        "video/mp4" => "mp4".into(),
        "video/webm" => "webm".into(),
        "video/quicktime" => "mov".into(),
        "audio/mp4" => "m4a".into(),
        "audio/mpeg" => "mp3".into(),
        "audio/webm" => "webm".into(),
        "image/jpeg" => "jpg".into(),
        "image/png" => "png".into(),
        "image/webp" => "webp".into(),
        other => {
            let after = other.rsplit('/').next().unwrap_or(other);
            match after {
                "mp4" | "webm" | "m4a" | "mp3" | "jpg" | "png" | "webp" | "mov" => after.into(),
                _ => "bin".into(),
            }
        }
    }
}
