use crate::cf::{self, Key, Owned};
use crate::ffi::{self, CgRect, ConnectionId, SpaceId};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::OnceLock;

/// An app with at least one normal window on a Space.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AppOnSpace {
  pub pid: i32,
  pub name: String,
  pub windows: usize,
}

/// Space id -> apps, each list ordered front-most window first.
pub type AppsBySpace = HashMap<SpaceId, Vec<AppOnSpace>>;

/// Windows smaller than this are helpers (status items, drag images, etc.).
const MIN_WINDOW_SIDE: i64 = 40;

struct Keys {
  number: Key,
  owner_pid: Key,
  owner_name: Key,
  layer: Key,
  bounds: Key,
  width: Key,
  height: Key,
  alpha: Key,
  name: Key,
  x: Key,
  y: Key,
}

fn keys() -> &'static Keys {
  static KEYS: OnceLock<Keys> = OnceLock::new();
  KEYS.get_or_init(|| Keys {
    number: Key::new("kCGWindowNumber"),
    owner_pid: Key::new("kCGWindowOwnerPID"),
    owner_name: Key::new("kCGWindowOwnerName"),
    layer: Key::new("kCGWindowLayer"),
    bounds: Key::new("kCGWindowBounds"),
    width: Key::new("Width"),
    height: Key::new("Height"),
    alpha: Key::new("kCGWindowAlpha"),
    name: Key::new("kCGWindowName"),
    x: Key::new("X"),
    y: Key::new("Y"),
  })
}

pub(crate) fn spaces_for_window(cid: ConnectionId, window_id: u32) -> Vec<SpaceId> {
  let ids = cf::new_number_array(&[window_id as i64]);
  unsafe {
    Owned::new(ffi::CGSCopySpacesForWindows(cid, ffi::ALL_SPACES_MASK, ids.as_ptr() as _) as _)
      .map(|spaces| {
        cf::array_items(spaces.as_ptr())
          .filter_map(|v| cf::number_i64(v))
          .map(|v| v as SpaceId)
          .collect()
      })
      .unwrap_or_default()
  }
}

/// Every window of one process with its layer, bounds, alpha and Spaces
/// (diagnostics).
pub fn describe_windows(cid: ConnectionId, pid: i32) -> Vec<String> {
  let k = keys();
  unsafe {
    let Some(list) = Owned::new(ffi::CGWindowListCopyWindowInfo(ffi::WINDOW_LIST_ALL, 0) as _)
    else {
      return Vec::new();
    };
    cf::array_items(list.as_ptr())
      .filter(|w| {
        cf::dict_get(*w, &k.owner_pid).and_then(|v| cf::number_i64(v)) == Some(pid as i64)
      })
      .map(|w| {
        let num = |key: &Key| {
          cf::dict_get(w, key)
            .and_then(|v| cf::number_i64(v))
            .unwrap_or(-1)
        };
        let bounds = cf::dict_get(w, &k.bounds);
        let b = |key: &str| {
          bounds
            .and_then(|d| cf::dict_get(d, &Key::new(key)))
            .and_then(|v| cf::number_i64(v))
            .unwrap_or(-1)
        };
        let id = num(&k.number) as u32;
        format!(
          "window {id} layer {} alpha {} at {},{} {}x{} spaces {:?}",
          num(&k.layer),
          num(&k.alpha),
          b("X"),
          b("Y"),
          b("Width"),
          b("Height"),
          spaces_for_window(cid, id)
        )
      })
      .collect()
  }
}

/// A normal (layer 0) window that belongs to exactly one Space.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WindowInfo {
  pub id: u32,
  pub pid: i32,
  pub app: String,
  /// Needs Screen Recording permission; `None` without it (and for the few
  /// windows that have no title).
  pub title: Option<String>,
  pub bounds: CgRect,
  pub space: SpaceId,
}

/// Normal windows on a single Space, front-most first. Windows on every Space
/// (sticky palettes, other overlays) say nothing about what a particular
/// desktop is for, background-tab windows are on none, and minimized windows
/// are left out.
pub(crate) fn windows(cid: ConnectionId, exclude_pid: i32) -> Vec<WindowInfo> {
  let k = keys();
  let mut out: Vec<WindowInfo> = unsafe {
    let Some(list) = Owned::new(ffi::CGWindowListCopyWindowInfo(
      ffi::WINDOW_LIST_ALL | ffi::WINDOW_LIST_EXCLUDE_DESKTOP,
      0,
    ) as _) else {
      return Vec::new();
    };
    cf::array_items(list.as_ptr())
      .filter_map(|w| {
        let num = |key: &Key| cf::dict_get(w, key).and_then(|v| cf::number_i64(v));
        let pid = num(&k.owner_pid)? as i32;
        if pid == exclude_pid || num(&k.layer)? != 0 || num(&k.alpha).unwrap_or(1) == 0 {
          return None;
        }
        let bounds = cf::dict_get(w, &k.bounds)?;
        let side = |key: &Key| {
          cf::dict_get(bounds, key)
            .and_then(|v| cf::number_i64(v))
            .unwrap_or(0)
        };
        if side(&k.width) < MIN_WINDOW_SIDE || side(&k.height) < MIN_WINDOW_SIDE {
          return None;
        }
        Some(WindowInfo {
          id: num(&k.number)? as u32,
          pid,
          app: cf::dict_get(w, &k.owner_name).and_then(|v| cf::string(v))?,
          title: cf::dict_get(w, &k.name)
            .and_then(|v| cf::string(v))
            .filter(|t| !t.is_empty()),
          bounds: CgRect {
            x: side(&k.x) as f64,
            y: side(&k.y) as f64,
            width: side(&k.width) as f64,
            height: side(&k.height) as f64,
          },
          space: 0,
        })
      })
      .collect()
  };
  // Minimized windows still report their Space; they are not "on" it for
  // our purposes, so they count as absent.
  out.retain(|window| !is_minimized(cid, window.id));
  out.retain_mut(|window| match spaces_for_window(cid, window.id)[..] {
    [space] => {
      window.space = space;
      true
    }
    _ => false,
  });
  out
}

/// WindowServer tag bit set while a window is minimized to the Dock
/// (verified: set by minimizing, clear on every normal window, including
/// windows on other Spaces).
const MINIMIZED_TAG: u64 = 1 << 60;

pub(crate) fn window_tags(cid: ConnectionId, window_id: u32) -> u64 {
  let mut tags = [0u32; 2];
  unsafe { ffi::CGSGetWindowTags(cid, window_id, tags.as_mut_ptr(), 64) };
  tags[0] as u64 | (tags[1] as u64) << 32
}

pub(crate) fn is_minimized(cid: ConnectionId, window_id: u32) -> bool {
  window_tags(cid, window_id) & MINIMIZED_TAG != 0
}

pub(crate) fn apps_by_space(windows: &[WindowInfo]) -> AppsBySpace {
  let mut by_space = AppsBySpace::new();
  for window in windows {
    let apps = by_space.entry(window.space).or_default();
    match apps.iter_mut().find(|a| a.pid == window.pid) {
      Some(app) => app.windows += 1,
      None => apps.push(AppOnSpace {
        pid: window.pid,
        name: window.app.clone(),
        windows: 1,
      }),
    }
  }
  by_space
}
