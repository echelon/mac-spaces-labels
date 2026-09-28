use crate::model::Placement;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// User preferences from the tray menu, persisted as JSON. A missing or
/// malformed file falls back to defaults; it must never block startup.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
  /// Read `corner` too, from settings saved before centered placements.
  #[serde(alias = "corner")]
  pub placement: Placement,
  pub show_apps: bool,
  /// Describe windows with the local vision model (when installed).
  pub vision: bool,
  /// Repository directory name -> how to show it ("artcraft" -> "ArtCraft
  /// Desktop"). Others are title-cased from the directory name.
  pub project_names: HashMap<String, String>,
}

impl Default for Settings {
  fn default() -> Self {
    Self {
      placement: Placement::default(),
      show_apps: true,
      vision: true,
      project_names: HashMap::new(),
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
