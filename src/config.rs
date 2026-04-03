use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub live: bool,
    pub staged: bool,
    pub collapse: Option<String>,
    pub theme: Option<String>,
}

impl Config {
    pub fn load() -> Self {
        let path = config_path();
        if !path.exists() {
            return Self::default();
        }
        match std::fs::read_to_string(&path) {
            Ok(content) => toml::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn collapse_level(&self) -> Option<crate::diff::model::CollapseLevel> {
        use crate::diff::model::CollapseLevel;
        match self.collapse.as_deref() {
            Some("tight") => Some(CollapseLevel::Tight),
            Some("scoped") => Some(CollapseLevel::Scoped),
            Some("expanded") => Some(CollapseLevel::Expanded),
            _ => None,
        }
    }
}

fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("better-diff")
        .join("config.toml")
}
