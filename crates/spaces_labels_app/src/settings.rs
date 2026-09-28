use crate::model::Corner;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// User preferences from the tray menu, persisted as JSON. A missing or
/// malformed file falls back to defaults; it must never block startup.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
  pub corner: Corner,
  pub show_apps: bool,
}

impl Default for Settings {
  fn default() -> Self {
    Self {
      corner: Corner::default(),
      show_apps: true,
    }
  }
}

impl Settings {
  pub fn load(path: &PathBuf) -> Self {
    std::fs::read(path)
      .ok()
      .and_then(|bytes| serde_json::from_slice(&bytes).ok())
      .unwrap_or_default()
  }

  pub fn save(&self, path: &PathBuf) {
    if let Some(dir) = path.parent() {
      let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_vec_pretty(self) {
      let _ = std::fs::write(path, json);
    }
  }
}
