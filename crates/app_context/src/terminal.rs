//! Ghostty windows, tabs and terminals (Ghostty's AppleScript dictionary,
//! 1.3+), and the tmux windows behind them.

use crate::osa;
use serde::Deserialize;
use std::collections::HashMap;
use std::process::Command;
use std::time::Duration;

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct GhosttyWindow {
  /// The selected tab's title (also the WindowServer title).
  #[serde(default)]
  pub name: String,
  #[serde(default)]
  pub tabs: Vec<GhosttyTab>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct GhosttyTab {
  #[serde(default)]
  pub name: String,
  #[serde(default)]
  pub selected: bool,
  #[serde(default)]
  pub terminals: Vec<GhosttyTerminal>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct GhosttyTerminal {
  #[serde(default)]
  pub name: String,
  #[serde(default)]
  pub cwd: Option<String>,
}

const GHOSTTY_SCRIPT: &str = r#"
const g = Application("Ghostty");
JSON.stringify(g.windows().map(w => ({
  name: w.name(),
  tabs: w.tabs().map(t => ({
    name: t.name(),
    selected: t.selected(),
    terminals: t.terminals().map(x => ({ name: x.name(), cwd: x.workingDirectory() })),
  })),
})))
"#;

/// Ghostty's windows, front-most first. Only call while Ghostty is running.
pub fn ghostty_windows() -> Result<Vec<GhosttyWindow>, String> {
  let json = osa::run_jxa(GHOSTTY_SCRIPT, Duration::from_secs(20))?;
  serde_json::from_str(json.trim()).map_err(|e| format!("ghostty reply: {e}"))
}

#[derive(Clone, Debug, PartialEq)]
pub struct TmuxWindow {
  pub index: u32,
  pub name: String,
  pub active: bool,
  pub command: String,
  pub path: String,
}

/// Session name -> its windows in index order. Empty when tmux is missing or
/// no server is running.
pub fn tmux_sessions() -> HashMap<String, Vec<TmuxWindow>> {
  // Apps launched from Finder get a minimal PATH, so look in the usual places.
  let Some(tmux) = [
    "/opt/homebrew/bin/tmux",
    "/usr/local/bin/tmux",
    "/usr/bin/tmux",
  ]
  .into_iter()
  .find(|p| std::path::Path::new(p).exists()) else {
    return HashMap::new();
  };
  let format = "#{session_name}\t#{window_index}\t#{window_name}\t#{window_active}\t#{pane_current_command}\t#{pane_current_path}";
  let Ok(output) = Command::new(tmux)
    .args(["list-windows", "-a", "-F", format])
    .output()
  else {
    return HashMap::new();
  };
  let mut sessions: HashMap<String, Vec<TmuxWindow>> = HashMap::new();
  for line in String::from_utf8_lossy(&output.stdout).lines() {
    let fields: Vec<&str> = line.split('\t').collect();
    if let [session, index, name, active, command, path] = fields[..] {
      sessions
        .entry(session.to_string())
        .or_default()
        .push(TmuxWindow {
          index: index.parse().unwrap_or(0),
          name: name.to_string(),
          active: active == "1",
          command: command.to_string(),
          path: path.to_string(),
        });
    }
  }
  sessions
}

/// The tmux session a terminal shows, from its title. tmux's `set-titles`
/// (e.g. `set-titles-string "#S / #W"`) puts the session name first; any
/// separator works as long as the title starts with a known session name.
pub fn session_for_title<'a>(
  title: &str,
  sessions: &'a HashMap<String, Vec<TmuxWindow>>,
) -> Option<&'a str> {
  sessions
    .keys()
    .filter(|name| {
      title == name.as_str()
        || title
          .strip_prefix(name.as_str())
          .is_some_and(|rest| rest.starts_with([' ', ':', '/', '-', '|']))
    })
    // Prefer the longest match so session "3" never claims "34 / zsh".
    .max_by_key(|name| name.len())
    .map(String::as_str)
}
