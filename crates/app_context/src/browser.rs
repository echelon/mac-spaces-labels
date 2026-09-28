//! Browser windows and tabs: Chrome via AppleScript, Firefox via its session
//! store. Both report window bounds, which match WindowServer bounds without
//! needing Screen Recording permission.

use crate::osa;
use serde::Deserialize;
use spaces_sys::CgRect;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BrowserWindow {
  /// The active tab's title.
  pub title: Option<String>,
  pub bounds: Option<CgRect>,
  /// Zoomed to fill its display; `bounds` are then the restore size.
  pub maximized: bool,
  pub tabs: Vec<Tab>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tab {
  pub title: String,
  pub url: String,
  pub active: bool,
}

// ---- Chrome ----------------------------------------------------------------

/// Bulk property reads: one Apple Event per property for all windows (and all
/// tabs), rather than one per tab, so hundreds of tabs still take ~100 ms.
const CHROME_SCRIPT: &str = r#"
const app = Application("Google Chrome");
JSON.stringify({
  names: app.windows.name(),
  bounds: app.windows.bounds(),
  active: app.windows.activeTabIndex(),
  titles: app.windows.tabs.title(),
  urls: app.windows.tabs.url(),
})
"#;

#[derive(Deserialize)]
struct ChromeReply {
  names: Vec<Option<String>>,
  bounds: Vec<Option<JsRect>>,
  active: Vec<Option<usize>>,
  titles: Vec<Vec<Option<String>>>,
  urls: Vec<Vec<Option<String>>>,
}

#[derive(Deserialize)]
struct JsRect {
  x: f64,
  y: f64,
  width: f64,
  height: f64,
}

/// Chrome's windows. Only call while Chrome is running: addressing a closed
/// app by name launches it.
pub fn chrome_windows() -> Result<Vec<BrowserWindow>, String> {
  let json = osa::run_jxa(CHROME_SCRIPT, Duration::from_secs(20))?;
  let reply: ChromeReply =
    serde_json::from_str(json.trim()).map_err(|e| format!("chrome reply: {e}"))?;
  Ok(
    (0..reply.names.len())
      .map(|w| {
        let active = reply.active.get(w).copied().flatten().unwrap_or(0);
        let titles = reply.titles.get(w).cloned().unwrap_or_default();
        let urls = reply.urls.get(w).cloned().unwrap_or_default();
        BrowserWindow {
          title: reply.names[w].clone(),
          maximized: false,
          bounds: reply
            .bounds
            .get(w)
            .and_then(|b| b.as_ref())
            .map(|b| CgRect {
              x: b.x,
              y: b.y,
              width: b.width,
              height: b.height,
            }),
          tabs: titles
            .into_iter()
            .enumerate()
            .map(|(i, title)| Tab {
              title: title.unwrap_or_default(),
              url: urls.get(i).cloned().flatten().unwrap_or_default(),
              active: i + 1 == active,
            })
            .collect(),
        }
      })
      .collect(),
  )
}

// ---- Firefox ---------------------------------------------------------------

/// Firefox (all channels) keeps open windows in each profile's
/// `sessionstore-backups/recovery.jsonlz4`, rewritten at most every 15 s
/// (`browser.sessionstore.interval`), so tab titles can lag that much. Files
/// are decoded only when their modification time changes.
#[derive(Default)]
pub struct FirefoxSessions {
  cache: HashMap<PathBuf, (SystemTime, Vec<BrowserWindow>)>,
}

#[derive(Deserialize)]
struct Session {
  #[serde(default)]
  windows: Vec<SessionWindow>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionWindow {
  #[serde(default)]
  tabs: Vec<SessionTab>,
  /// 1-based.
  #[serde(default)]
  selected: usize,
  screen_x: Option<f64>,
  screen_y: Option<f64>,
  width: Option<f64>,
  height: Option<f64>,
  sizemode: Option<String>,
}

#[derive(Deserialize)]
struct SessionTab {
  #[serde(default)]
  entries: Vec<SessionEntry>,
  /// 1-based index of the current history entry.
  #[serde(default)]
  index: usize,
  #[serde(default)]
  hidden: bool,
}

#[derive(Deserialize)]
struct SessionEntry {
  #[serde(default)]
  url: String,
  #[serde(default)]
  title: String,
}

impl FirefoxSessions {
  /// Windows from every profile's session. Stale profiles (Firefox closed)
  /// simply fail to match any live window.
  pub fn windows(&mut self) -> Vec<BrowserWindow> {
    let Some(home) = std::env::var_os("HOME") else {
      return Vec::new();
    };
    let profiles = Path::new(&home).join("Library/Application Support/Firefox/Profiles");
    let Ok(dirs) = std::fs::read_dir(profiles) else {
      return Vec::new();
    };
    let mut out = Vec::new();
    for dir in dirs.flatten() {
      let path = dir.path().join("sessionstore-backups/recovery.jsonlz4");
      let Ok(modified) = std::fs::metadata(&path).and_then(|m| m.modified()) else {
        continue;
      };
      if !matches!(self.cache.get(&path), Some((at, _)) if *at == modified) {
        let windows = read_session(&path).unwrap_or_default();
        self.cache.insert(path.clone(), (modified, windows));
      }
      out.extend(self.cache[&path].1.iter().cloned());
    }
    out
  }
}

fn read_session(path: &Path) -> Option<Vec<BrowserWindow>> {
  let bytes = std::fs::read(path).ok()?;
  // "mozLz40\0", little-endian decompressed size, then one LZ4 block.
  if bytes.len() < 12 || &bytes[..8] != b"mozLz40\0" {
    return None;
  }
  let size = u32::from_le_bytes(bytes[8..12].try_into().ok()?) as usize;
  let json = lz4_flex::block::decompress(&bytes[12..], size).ok()?;
  let session: Session = serde_json::from_slice(&json).ok()?;
  Some(
    session
      .windows
      .into_iter()
      .map(|w| {
        let tabs: Vec<Tab> = w
          .tabs
          .iter()
          .enumerate()
          .filter(|(_, t)| !t.hidden)
          .filter_map(|(i, t)| {
            let entry = t.entries.get(t.index.max(1) - 1).or(t.entries.last())?;
            Some(Tab {
              title: entry.title.clone(),
              url: entry.url.clone(),
              active: i + 1 == w.selected,
            })
          })
          .collect();
        let bounds = match (w.screen_x, w.screen_y, w.width, w.height) {
          (Some(x), Some(y), Some(width), Some(height)) => Some(CgRect {
            x,
            y,
            width,
            height,
          }),
          _ => None,
        };
        BrowserWindow {
          title: tabs.iter().find(|t| t.active).map(|t| t.title.clone()),
          bounds,
          maximized: w.sizemode.as_deref() == Some("maximized"),
          tabs,
        }
      })
      .collect(),
  )
}

/// `https://www.example.com/a?b` -> `example.com`.
pub fn host(url: &str) -> Option<String> {
  let rest = url.split_once("://")?.1;
  let host = rest.split(['/', '?', '#']).next()?.rsplit('@').next()?;
  let host = host.strip_prefix("www.").unwrap_or(host);
  (!host.is_empty()).then(|| host.to_string())
}
