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
  /// Show the label on arriving at a desktop, then fade it out.
  pub auto_hide: bool,
  /// How long a label stays fully visible after arriving, in ms.
  pub show_ms: u32,
  /// How long it takes to fade out, in ms.
  pub fade_ms: u32,
  /// After leaving a desktop, wait this long (past the slide animation)
  /// before resetting its hidden label for the next visit, in ms.
  pub rearm_ms: u32,
  /// How long a label waits after the pointer leaves it before fading, in ms.
  pub linger_ms: u32,
  /// Opacity of the panel behind the text, 0–1.
  pub panel_opacity: f32,
}

impl Default for Settings {
  fn default() -> Self {
    Self {
      placement: Placement::default(),
      show_apps: true,
      vision: true,
      project_names: HashMap::new(),
      auto_hide: true,
      show_ms: 1100,
      fade_ms: 700,
      rearm_ms: 600,
      linger_ms: 400,
      panel_opacity: 0.85,
    }
  }
}

/// The label's animation and look, as the page needs them.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Timing {
  pub show_ms: u32,
  pub fade_ms: u32,
  pub rearm_ms: u32,
  pub linger_ms: u32,
  pub panel_opacity: f32,
}

impl Settings {
  pub fn timing(&self) -> Timing {
    Timing {
      show_ms: self.show_ms,
      fade_ms: self.fade_ms,
      rearm_ms: self.rearm_ms,
      linger_ms: self.linger_ms,
      panel_opacity: self.panel_opacity.clamp(0.0, 1.0),
    }
  }

  pub fn modified(path: &PathBuf) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
  }

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
