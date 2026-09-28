//! What is happening inside each window, grouped by Space.
//!
//! WindowServer (via `spaces-sys`) says which windows are on which Space. The
//! apps themselves say what those windows contain:
//!
//! | App | Source | Matched to its window by |
//! | --- | --- | --- |
//! | Chrome | AppleScript (Automation permission) | bounds, then title |
//! | Firefox (any channel) | profile session store (no permission) | bounds, then title |
//! | Ghostty | AppleScript, 1.3+ (Automation permission) | title (needs Screen Recording) |
//! | tmux | `tmux list-windows` | the terminal title's session name |
//! | anything else | window title (Screen Recording permission) | — |
//!
//! Every source is optional: a failure leaves that app with plain window
//! titles and is reported in [`Contexts::errors`].

#![cfg(target_os = "macos")]

pub mod browser;
mod osa;
pub mod terminal;

use browser::{BrowserWindow, FirefoxSessions};
use serde::Serialize;
use spaces_sys::{CgRect, SpaceId, WindowInfo};
use std::collections::{HashMap, HashSet};
use terminal::{GhosttyWindow, TmuxWindow};

pub const CHROME: &str = "Google Chrome";
pub const GHOSTTY: &str = "Ghostty";
pub const FIREFOX_APPS: [&str; 3] = ["Firefox", "Firefox Developer Edition", "Firefox Nightly"];

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct SpaceContext {
  /// Front-most app first.
  pub apps: Vec<AppContext>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct AppContext {
  pub name: String,
  /// Front-most window first.
  pub windows: Vec<WindowContext>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct WindowContext {
  pub title: Option<String>,
  /// Browser tabs or terminal tabs; empty for other apps.
  pub tabs: Vec<Entry>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Entry {
  pub title: String,
  /// Site for tabs, working directory for terminals.
  pub detail: Option<String>,
  pub active: bool,
  /// tmux windows (or split terminals) inside a terminal tab.
  pub children: Vec<Entry>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Contexts {
  pub by_space: HashMap<SpaceId, SpaceContext>,
  pub titles_readable: bool,
  pub errors: Vec<String>,
}

/// Holds caches between collections (decoded Firefox sessions).
#[derive(Default)]
pub struct Collector {
  firefox: FirefoxSessions,
}

impl Collector {
  /// Slow (AppleScript round trips of ~100 ms): call off the UI path.
  /// `windows` must come from `Spaces::windows`, front-most first.
  pub fn collect(&mut self, windows: &[WindowInfo]) -> Contexts {
    let running = |names: &[&str]| windows.iter().any(|w| names.contains(&w.app.as_str()));
    let mut errors = Vec::new();
    let chrome: Vec<BrowserWindow> = if running(&[CHROME]) {
      take(&mut errors, CHROME, browser::chrome_windows())
    } else {
      Vec::new()
    };
    let ghostty: Vec<GhosttyWindow> = if running(&[GHOSTTY]) {
      take(&mut errors, GHOSTTY, terminal::ghostty_windows())
    } else {
      Vec::new()
    };
    let firefox = if running(&FIREFOX_APPS) {
      self.firefox.windows()
    } else {
      Vec::new()
    };
    let tmux = if ghostty.is_empty() {
      HashMap::new()
    } else {
      terminal::tmux_sessions()
    };
    let titles_readable = windows.iter().any(|w| w.title.is_some());

    let of = |names: &[&str]| -> Vec<&WindowInfo> {
      windows
        .iter()
        .filter(|w| names.contains(&w.app.as_str()))
        .collect()
    };
    let chrome_match = match_browsers(&of(&[CHROME]), &chrome);
    let firefox_match = match_browsers(&of(&FIREFOX_APPS), &firefox);
    let ghostty_match = match_ghostty(&of(&[GHOSTTY]), &ghostty);

    let mut by_space: HashMap<SpaceId, SpaceContext> = HashMap::new();
    for window in windows {
      let tabs = if let Some(&i) = chrome_match.get(&window.id) {
        browser_tabs(&chrome[i])
      } else if let Some(&i) = firefox_match.get(&window.id) {
        browser_tabs(&firefox[i])
      } else if let Some(&i) = ghostty_match.get(&window.id) {
        ghostty_tabs(&ghostty[i], &tmux)
      } else {
        Vec::new()
      };
      let space = by_space.entry(window.space).or_default();
      let app = match space.apps.iter_mut().position(|a| a.name == window.app) {
        Some(i) => &mut space.apps[i],
        None => {
          space.apps.push(AppContext {
            name: window.app.clone(),
            windows: Vec::new(),
          });
          space.apps.last_mut().unwrap()
        }
      };
      app.windows.push(WindowContext {
        title: window.title.clone(),
        tabs,
      });
    }
    Contexts {
      by_space,
      titles_readable,
      errors,
    }
  }
}

/// A source's windows, or none (recording why) when it failed.
fn take<T>(errors: &mut Vec<String>, source: &str, result: Result<Vec<T>, String>) -> Vec<T> {
  result.unwrap_or_else(|e| {
    errors.push(format!("{source}: {e}"));
    Vec::new()
  })
}

fn close(a: &CgRect, b: &CgRect) -> bool {
  const SLACK: f64 = 3.0;
  (a.x - b.x).abs() <= SLACK
    && (a.y - b.y).abs() <= SLACK
    && (a.width - b.width).abs() <= SLACK
    && (a.height - b.height).abs() <= SLACK
}

/// Whether a window spans a display's full width down to its bottom edge
/// (a zoomed window; the menu bar and Dock may take the rest).
fn fills_display(window: &CgRect, displays: &[CgRect]) -> bool {
  displays.iter().any(|d| {
    (window.x - d.x).abs() <= 3.0
      && (window.width - d.width).abs() <= 3.0
      && (window.y + window.height - (d.y + d.height)).abs() <= 100.0
  })
}

/// WindowServer window id -> index into `browsers`. Bounds identify most
/// windows (a zoomed window matches one filling its display); the title
/// breaks ties and covers windows whose bounds the source did not report. A
/// single window left over on each side is paired last: a window resized
/// since Firefox last saved its session.
fn match_browsers(windows: &[&WindowInfo], browsers: &[BrowserWindow]) -> HashMap<u32, usize> {
  let displays = spaces_sys::display_rects();
  // Private windows are never saved to a session, so they can only match
  // wrongly (a private window and a zoomed one both fill the display).
  let private = |w: &WindowInfo| {
    w.title
      .as_deref()
      .is_some_and(|t| t.contains("Private Browsing"))
  };
  let windows: Vec<&WindowInfo> = windows.iter().copied().filter(|w| !private(w)).collect();
  let mut used = HashSet::new();
  let mut out = HashMap::new();
  for window in &windows {
    let best = browsers
      .iter()
      .enumerate()
      .filter(|(i, _)| !used.contains(i))
      .map(|(i, b)| {
        let bounds = if b.maximized {
          fills_display(&window.bounds, &displays)
        } else {
          b.bounds.is_some_and(|r| close(&r, &window.bounds))
        };
        let title = matches!((&window.title, &b.title), (Some(w), Some(t)) if !t.is_empty() && w.starts_with(t.as_str()));
        (i, bounds as u8 * 2 + title as u8)
      })
      .filter(|(_, score)| *score > 0)
      .max_by_key(|(_, score)| *score);
    if let Some((i, _)) = best {
      used.insert(i);
      out.insert(window.id, i);
    }
  }
  let left_windows: Vec<_> = windows
    .iter()
    .filter(|w| !out.contains_key(&w.id))
    .collect();
  let left_browsers: Vec<_> = (0..browsers.len()).filter(|i| !used.contains(i)).collect();
  if let ([window], [browser]) = (&left_windows[..], &left_browsers[..]) {
    out.insert(window.id, *browser);
  }
  out
}

/// Ghostty reports no bounds or window numbers, so its windows are matched
/// by title (a Ghostty window's title is its selected tab's). Without Screen
/// Recording there are no titles and no match: the z-orders of the two lists
/// disagree, and a wrong tmux session is worse than none.
fn match_ghostty(windows: &[&WindowInfo], ghostty: &[GhosttyWindow]) -> HashMap<u32, usize> {
  let mut used = HashSet::new();
  let mut out = HashMap::new();
  for window in windows {
    let found = ghostty
      .iter()
      .enumerate()
      .find(|(i, g)| !used.contains(i) && window.title.as_deref() == Some(g.name.as_str()));
    if let Some((i, _)) = found {
      used.insert(i);
      out.insert(window.id, i);
    }
  }
  out
}

fn browser_tabs(window: &BrowserWindow) -> Vec<Entry> {
  window
    .tabs
    .iter()
    .map(|tab| Entry {
      title: if tab.title.is_empty() {
        tab.url.clone()
      } else {
        tab.title.clone()
      },
      detail: browser::host(&tab.url),
      active: tab.active,
      children: Vec::new(),
    })
    .collect()
}

fn ghostty_tabs(window: &GhosttyWindow, tmux: &HashMap<String, Vec<TmuxWindow>>) -> Vec<Entry> {
  window
    .tabs
    .iter()
    .map(|tab| {
      let split = tab.terminals.len() > 1;
      let children = tab
        .terminals
        .iter()
        .flat_map(
          |terminal| match terminal::session_for_title(&terminal.name, tmux) {
            Some(session) => tmux[session]
              .iter()
              .map(|w| Entry {
                title: format!("{session}:{} {}", w.index, w.name),
                detail: Some(tilde(&w.path)),
                active: w.active,
                children: Vec::new(),
              })
              .collect(),
            None if split => vec![Entry {
              title: terminal.name.clone(),
              detail: terminal.cwd.as_deref().map(tilde),
              active: false,
              children: Vec::new(),
            }],
            None => Vec::new(),
          },
        )
        .collect();
      Entry {
        title: tab.name.clone(),
        detail: tab
          .terminals
          .first()
          .and_then(|t| t.cwd.as_deref())
          .map(tilde),
        active: tab.selected,
        children,
      }
    })
    .collect()
}

fn tilde(path: &str) -> String {
  match std::env::var("HOME") {
    Ok(home) if !home.is_empty() && path.starts_with(&home) => format!("~{}", &path[home.len()..]),
    _ => path.to_string(),
  }
}
