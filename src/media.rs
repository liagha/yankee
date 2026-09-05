//! shared model every adapter returns

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
    pub assets: Vec<Asset>,
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