//! config from config.toml next to the binary

use std::fs;

use serde::Deserialize;

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Config {
    pub dir: Option<String>,
    pub proxy: Option<String>,
}

impl Config {
    pub fn load() -> Self {
        let path = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("config.toml")))
            .unwrap_or_else(|| "config.toml".into());
        let cfg = fs::read_to_string(path)
            .ok()
            .and_then(|raw| toml::from_str::<Config>(&raw).ok())
            .unwrap_or_default();
        cfg.with_defaults()
    }

    fn with_defaults(mut self) -> Self {
        if self.dir.is_none() {
            self.dir = Some("downloads".into());
        }
        self
    }
}