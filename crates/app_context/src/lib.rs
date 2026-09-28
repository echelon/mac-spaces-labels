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
//! | Claude Code, Codex | process tree + `tmux capture-pane` (see [`activity`]) | tmux pane |
//! | local dev servers | `lsof` listeners + their working directory | `localhost:PORT` tab URL |
//! | anything else | window title (Screen Recording permission) | — |
//!
//! Every source is optional: a failure leaves that app with plain window
//! titles and is reported in [`Contexts::errors`].

#![cfg(target_os = "macos")]

pub mod activity;
pub mod browser;
mod osa;
pub mod terminal;

use activity::{Agent, AgentState, Pane, Server};
use browser::{BrowserWindow, FirefoxSessions};
use serde::Serialize;
use spaces_sys::{CgRect, SpaceId, WindowInfo};
use std::collections::{HashMap, HashSet};
use terminal::{GhosttyWindow, TmuxWindow};

pub const CHROME: &str = "Google Chrome";
pub const GHOSTTY: &str = "Ghostty";
pub const FIREFOX_APPS: [&str; 3] = ["Firefox", "Firefox Developer Edition", "Firefox Nightly"];
/// JetBrains IDEs title windows "project – file".
const JETBRAINS: [&str; 10] = [
  "RustRover",
  "IntelliJ IDEA",
  "WebStorm",
  "PyCharm",
  "GoLand",
  "CLion",
  "PhpStorm",
  "Rider",
  "DataGrip",
  "RubyMine",
];

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct SpaceContext {
  /// What this desktop is for: the projects its windows point at, most
  /// active first. The "semantic grouping" of the desktop.
  pub projects: Vec<Project>,
  /// Front-most app first.
  pub apps: Vec<AppContext>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Project {
  pub name: String,
  pub path: Option<String>,
  /// Evidence, strongest first: "Claude working", "dev site :4201", "RustRover", "terminal".
  pub signals: Vec<String>,
  #[serde(skip)]
  score: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct AppContext {
  pub name: String,
  /// Front-most window first.
  pub windows: Vec<WindowContext>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct WindowContext {
  /// WindowServer window number (for screenshots).
  pub id: u32,
  pub title: Option<String>,
  /// Browser tabs or terminal tabs; empty for other apps.
  pub tabs: Vec<Entry>,
  /// A local vision model's description of the window, filled in by the app.
  pub vision: Option<String>,
  #[serde(skip)]
  pub width: f64,
  #[serde(skip)]
  pub height: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Entry {
  pub title: String,
  /// Site for tabs, working directory for terminals.
  pub detail: Option<String>,
  pub active: bool,
  /// A coding agent's state, e.g. "Claude working · Burrowing… (2m 39s)".
  pub status: Option<String>,
  /// Longer context: the agent's recap or task, or which local server (and
  /// directory) serves a localhost tab.
  pub note: Option<String>,
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

/// tmux and the agents running in it.
#[derive(Default)]
struct Terminals {
  sessions: HashMap<String, Vec<TmuxWindow>>,
  panes: Vec<Pane>,
  /// (session, window index) -> the agent in that tmux window.
  agents: HashMap<(String, u32), Agent>,
}

impl Terminals {
  fn read() -> Self {
    let sessions = terminal::tmux_sessions();
    let panes = if sessions.is_empty() {
      Vec::new()
    } else {
      activity::tmux_panes()
    };
    let tree = if panes.is_empty() {
      HashMap::new()
    } else {
      activity::process_tree()
    };
    let mut agents = HashMap::new();
    for pane in &panes {
      if let Some(kind) = activity::agent_kind(pane, &tree) {
        agents
          .entry((pane.session.clone(), pane.window))
          .or_insert_with(|| activity::agent_status(kind, pane));
      }
    }
    Self {
      sessions,
      panes,
      agents,
    }
  }
}

fn agent_status(agent: &Agent) -> String {
  let state = match agent.state {
    AgentState::Working => "working",
    AgentState::Waiting => "waiting for you",
    AgentState::Idle => "idle",
  };
  let mut status = format!("{} {state}", agent.kind.name());
  if let (AgentState::Working, Some(activity)) = (agent.state, &agent.activity) {
    status.push_str(" · ");
    status.push_str(activity);
  }
  status
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
    let terminals = if ghostty.is_empty() {
      Terminals::default()
    } else {
      Terminals::read()
    };
    let ports: Vec<u16> = chrome
      .iter()
      .chain(&firefox)
      .flat_map(|w| &w.tabs)
      .filter_map(|t| activity::local_port(&t.url))
      .collect::<HashSet<_>>()
      .into_iter()
      .collect();
    let servers = activity::servers(&ports);
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
    let mut projects: HashMap<SpaceId, Projects> = HashMap::new();
    for window in windows {
      let found = projects.entry(window.space).or_default();
      let tabs = if let Some(&i) = chrome_match.get(&window.id) {
        browser_tabs(&chrome[i], &servers, found)
      } else if let Some(&i) = firefox_match.get(&window.id) {
        browser_tabs(&firefox[i], &servers, found)
      } else if let Some(&i) = ghostty_match.get(&window.id) {
        ghostty_tabs(&ghostty[i], &terminals, found)
      } else {
        Vec::new()
      };
      if JETBRAINS.contains(&window.app.as_str()) {
        if let Some((project, _)) = window.title.as_deref().and_then(|t| t.split_once(" – ")) {
          found.add_name(project.trim(), &window.app, 3);
        }
      }
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
        id: window.id,
        title: window.title.clone(),
        tabs,
        vision: None,
        width: window.bounds.width,
        height: window.bounds.height,
      });
    }
    for (space, found) in projects {
      by_space.entry(space).or_default().projects = found.finish();
    }
    Contexts {
      by_space,
      titles_readable,
      errors,
    }
  }
}

/// Accumulates project evidence for one Space.
#[derive(Default)]
struct Projects {
  list: Vec<Project>,
}

impl Projects {
  /// Evidence from a directory: grouped by repository root.
  fn add_path(&mut self, path: &str, signal: &str, score: u32) {
    let root = activity::project_root(path);
    let name = root
      .file_name()
      .map(|n| n.to_string_lossy().into_owned())
      .unwrap_or_default();
    self.add(name, Some(tilde(&root.to_string_lossy())), signal, score);
  }

  /// Evidence from a name only (an IDE window title).
  fn add_name(&mut self, name: &str, signal: &str, score: u32) {
    self.add(name.to_string(), None, signal, score);
  }

  fn add(&mut self, name: String, path: Option<String>, signal: &str, score: u32) {
    if name.is_empty() {
      return;
    }
    let found = self.list.iter_mut().find(|p| match (&p.path, &path) {
      (Some(a), Some(b)) => a == b,
      _ => p.name == name,
    });
    let project = match found {
      Some(p) => p,
      None => {
        self.list.push(Project {
          name,
          path: None,
          signals: Vec::new(),
          score: 0,
        });
        self.list.last_mut().unwrap()
      }
    };
    if project.path.is_none() {
      project.path = path;
    }
    project.score += score;
    if !project.signals.iter().any(|s| s == signal) {
      project.signals.push(signal.to_string());
    }
  }

  fn finish(mut self) -> Vec<Project> {
    self.list.sort_by_key(|p| std::cmp::Reverse(p.score));
    self.list
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

fn browser_tabs(
  window: &BrowserWindow,
  servers: &HashMap<u16, Server>,
  found: &mut Projects,
) -> Vec<Entry> {
  window
    .tabs
    .iter()
    .map(|tab| {
      let server = activity::local_port(&tab.url).and_then(|port| servers.get(&port));
      if let Some(cwd) = server.and_then(|s| s.cwd.as_deref()) {
        let signal = format!("dev site :{}", server.unwrap().port);
        found.add_path(cwd, &signal, if tab.active { 4 } else { 2 });
      }
      Entry {
        title: if tab.title.is_empty() {
          tab.url.clone()
        } else {
          tab.title.clone()
        },
        detail: browser::host(&tab.url),
        active: tab.active,
        note: server.map(|s| match &s.cwd {
          Some(cwd) => format!("served by {} from {}", s.command, tilde(cwd)),
          None => format!("served by {}", s.command),
        }),
        ..Default::default()
      }
    })
    .collect()
}

fn ghostty_tabs(window: &GhosttyWindow, terminals: &Terminals, found: &mut Projects) -> Vec<Entry> {
  window
    .tabs
    .iter()
    .map(|tab| {
      let split = tab.terminals.len() > 1;
      let mut children = Vec::new();
      for terminal in &tab.terminals {
        match terminal::session_for_title(&terminal.name, &terminals.sessions) {
          Some(session) => {
            for w in &terminals.sessions[session] {
              let agent = terminals.agents.get(&(session.to_string(), w.index));
              children.push(Entry {
                title: format!("{session}:{} {}", w.index, w.name),
                detail: Some(tilde(&w.path)),
                active: w.active,
                status: agent.map(agent_status),
                note: agent.and_then(|a| a.note.clone()),
                ..Default::default()
              });
            }
            // Every pane of the session is evidence for its project.
            for pane in terminals.panes.iter().filter(|p| p.session == session) {
              match terminals.agents.get(&(pane.session.clone(), pane.window)) {
                Some(agent) => {
                  let score = match agent.state {
                    AgentState::Working => 10,
                    AgentState::Waiting => 8,
                    AgentState::Idle => 5,
                  };
                  let state = agent_status(agent);
                  let short = state.split(" · ").next().unwrap_or(&state);
                  found.add_path(&agent.path, short, score);
                }
                None => found.add_path(&pane.path, "terminal", 1),
              }
            }
          }
          None => {
            if let Some(cwd) = &terminal.cwd {
              found.add_path(cwd, "terminal", 1);
            }
            if split {
              children.push(Entry {
                title: terminal.name.clone(),
                detail: terminal.cwd.as_deref().map(tilde),
                ..Default::default()
              });
            }
          }
        }
      }
      Entry {
        title: tab.name.clone(),
        detail: tab
          .terminals
          .first()
          .and_then(|t| t.cwd.as_deref())
          .map(tilde),
        active: tab.selected,
        children,
        ..Default::default()
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
