//! Process interrogation: which coding agents run in which tmux panes and
//! what they are doing, and which local servers back `localhost` browser tabs.
//! Exact and cheap (a few `ps`/`tmux`/`lsof` calls), unlike looking at pixels.

use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Debug, PartialEq)]
pub struct Pane {
  pub session: String,
  pub window: u32,
  pub pane: u32,
  pub pid: i32,
  pub command: String,
  pub path: String,
  pub active: bool,
}

impl Pane {
  pub fn target(&self) -> String {
    format!("{}:{}.{}", self.session, self.window, self.pane)
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
  Claude,
  Codex,
}

impl AgentKind {
  pub fn name(self) -> &'static str {
    match self {
      AgentKind::Claude => "Claude",
      AgentKind::Codex => "Codex",
    }
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
  Working,
  /// A question or permission prompt is on screen.
  Waiting,
  Idle,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Agent {
  pub kind: AgentKind,
  pub state: AgentState,
  pub path: String,
  /// The live status line, e.g. "Burrowing… (2m 39s · ↓ 7.0k tokens)".
  pub activity: Option<String>,
  /// The agent's own summary: Claude's recap, or Codex's task title.
  pub note: Option<String>,
}

pub fn tmux_binary() -> Option<&'static str> {
  // Apps launched from Finder get a minimal PATH, so look in the usual places.
  [
    "/opt/homebrew/bin/tmux",
    "/usr/local/bin/tmux",
    "/usr/bin/tmux",
  ]
  .into_iter()
  .find(|p| Path::new(p).exists())
}

pub fn tmux_panes() -> Vec<Pane> {
  let Some(tmux) = tmux_binary() else {
    return Vec::new();
  };
  let format = "#{session_name}\t#{window_index}\t#{pane_index}\t#{pane_pid}\t#{pane_current_command}\t#{pane_current_path}\t#{pane_active}";
  let Ok(output) = Command::new(tmux)
    .args(["list-panes", "-a", "-F", format])
    .output()
  else {
    return Vec::new();
  };
  String::from_utf8_lossy(&output.stdout)
    .lines()
    .filter_map(|line| {
      let f: Vec<&str> = line.split('\t').collect();
      let [session, window, pane, pid, command, path, active] = f[..] else {
        return None;
      };
      Some(Pane {
        session: session.into(),
        window: window.parse().ok()?,
        pane: pane.parse().ok()?,
        pid: pid.parse().ok()?,
        command: command.into(),
        path: path.into(),
        active: active == "1",
      })
    })
    .collect()
}

/// Parent pid -> children's (pid, executable name). One `ps` for everything.
pub fn process_tree() -> HashMap<i32, Vec<(i32, String)>> {
  let Ok(output) = Command::new("/bin/ps")
    .args(["-axo", "pid=,ppid=,comm="])
    .output()
  else {
    return HashMap::new();
  };
  let mut tree: HashMap<i32, Vec<(i32, String)>> = HashMap::new();
  for line in String::from_utf8_lossy(&output.stdout).lines() {
    let mut parts = line.split_whitespace();
    let (Some(pid), Some(ppid)) = (parts.next(), parts.next()) else {
      continue;
    };
    let comm: String = parts.collect::<Vec<_>>().join(" ");
    let name = comm.rsplit('/').next().unwrap_or(&comm).to_string();
    if let (Ok(pid), Ok(ppid)) = (pid.parse(), ppid.parse()) {
      tree.entry(ppid).or_default().push((pid, name));
    }
  }
  tree
}

/// The agent running in a pane, if any: its process or a descendant is
/// `claude` or `codex` (Claude Code retitles itself to its version number, so
/// the pane's command alone is not enough).
pub fn agent_kind(pane: &Pane, tree: &HashMap<i32, Vec<(i32, String)>>) -> Option<AgentKind> {
  let classify = |name: &str| match name {
    "claude" => Some(AgentKind::Claude),
    "codex" => Some(AgentKind::Codex),
    _ => None,
  };
  if let Some(kind) = classify(&pane.command) {
    return Some(kind);
  }
  let mut stack = vec![pane.pid];
  let mut seen = 0;
  while let Some(pid) = stack.pop() {
    seen += 1;
    if seen > 200 {
      break;
    }
    for (child, name) in tree.get(&pid).into_iter().flatten() {
      if let Some(kind) = classify(name) {
        return Some(kind);
      }
      stack.push(*child);
    }
  }
  None
}

/// Reads the agent's screen (`tmux capture-pane`, ~8 ms).
pub fn agent_status(kind: AgentKind, pane: &Pane) -> Agent {
  let text = tmux_binary()
    .and_then(|tmux| {
      Command::new(tmux)
        .args(["capture-pane", "-p", "-t", &pane.target()])
        .output()
        .ok()
    })
    .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    .unwrap_or_default();
  parse_screen(kind, &pane.path, &text)
}

pub fn parse_screen(kind: AgentKind, path: &str, text: &str) -> Agent {
  let lines: Vec<&str> = text
    .lines()
    .map(str::trim_end)
    .filter(|l| !l.trim().is_empty())
    .collect();
  let tail = &lines[lines.len().saturating_sub(25)..];
  let working = tail.iter().any(|l| l.contains("esc to interrupt"));
  let waiting = !working
    && tail.iter().any(|l| {
      l.contains("Do you want to") || l.contains("Would you like to") || l.contains("❯ 1. Yes")
    });
  let state = if working {
    AgentState::Working
  } else if waiting {
    AgentState::Waiting
  } else {
    AgentState::Idle
  };
  let (activity, note) = match kind {
    AgentKind::Claude => (claude_spinner(tail), claude_recap(&lines)),
    AgentKind::Codex => (
      tail
        .iter()
        .rev()
        .find(|l| l.contains("Working (") || l.contains("Worked for"))
        .map(|l| clean(l)),
      codex_task(tail),
    ),
  };
  Agent {
    kind,
    state,
    path: path.to_string(),
    activity,
    note,
  }
}

/// Claude's status line: a spinner glyph, a gerund, an ellipsis, a timer.
fn claude_spinner(tail: &[&str]) -> Option<String> {
  tail.iter().rev().find_map(|l| {
    let t = l.trim_start();
    let first = t.chars().next()?;
    ("✶✻✽✢✳·*⏺✺✹".contains(first) && t.contains('…') && t.contains('('))
      .then(|| clean(t.trim_start_matches(|c: char| !c.is_alphanumeric())))
  })
}

/// Claude's "※ recap:" paragraph, the most recent one.
fn claude_recap(lines: &[&str]) -> Option<String> {
  let start = lines.iter().rposition(|l| l.contains("※ recap:"))?;
  let mut text = lines[start].split_once("recap:")?.1.trim().to_string();
  for line in &lines[start + 1..] {
    let t = line.trim();
    if !line.starts_with("  ") || t.starts_with('─') || t.starts_with('❯') {
      break;
    }
    text.push(' ');
    text.push_str(t);
  }
  let text = text.replace("(disable recaps in /config)", "");
  Some(clean(&text)).filter(|t| !t.is_empty())
}

/// Codex's footer: "model · ~/path · Task title".
fn codex_task(tail: &[&str]) -> Option<String> {
  tail.iter().rev().find_map(|l| {
    let parts: Vec<&str> = l.split(" · ").collect();
    (parts.len() >= 3 && parts[1].trim_start().starts_with('~'))
      .then(|| clean(parts[2..].join(" · ").as_str()))
  })
}

fn clean(text: &str) -> String {
  text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A local server: who listens on a port, and from which directory.
#[derive(Clone, Debug, PartialEq)]
pub struct Server {
  pub port: u16,
  pub command: String,
  pub cwd: Option<String>,
}

/// Servers for the given ports (from `localhost` tabs). Two `lsof` calls.
pub fn servers(ports: &[u16]) -> HashMap<u16, Server> {
  if ports.is_empty() {
    return HashMap::new();
  }
  let Ok(output) = Command::new("/usr/sbin/lsof")
    .args(["-nP", "-iTCP", "-sTCP:LISTEN", "-Fpcn"])
    .output()
  else {
    return HashMap::new();
  };
  let mut found: HashMap<u16, (i32, String)> = HashMap::new();
  let (mut pid, mut command) = (0, String::new());
  for line in String::from_utf8_lossy(&output.stdout).lines() {
    let (tag, value) = line.split_at(1.min(line.len()));
    match tag {
      "p" => pid = value.parse().unwrap_or(0),
      "c" => command = value.to_string(),
      "n" => {
        if let Some(port) = value.rsplit(':').next().and_then(|p| p.parse::<u16>().ok()) {
          if ports.contains(&port) {
            found.entry(port).or_insert((pid, command.clone()));
          }
        }
      }
      _ => {}
    }
  }
  let pids: Vec<String> = found.values().map(|(pid, _)| pid.to_string()).collect();
  let mut cwds: HashMap<i32, String> = HashMap::new();
  if !pids.is_empty() {
    if let Ok(output) = Command::new("/usr/sbin/lsof")
      .args(["-nP", "-a", "-d", "cwd", "-Fpn", "-p", &pids.join(",")])
      .output()
    {
      let mut pid = 0;
      for line in String::from_utf8_lossy(&output.stdout).lines() {
        let (tag, value) = line.split_at(1.min(line.len()));
        match tag {
          "p" => pid = value.parse().unwrap_or(0),
          "n" => {
            cwds.insert(pid, value.to_string());
          }
          _ => {}
        }
      }
    }
  }
  found
    .into_iter()
    .map(|(port, (pid, command))| {
      (
        port,
        Server {
          port,
          command,
          cwd: cwds.get(&pid).cloned(),
        },
      )
    })
    .collect()
}

/// The port of a local URL (`http://localhost:4201/…`).
pub fn local_port(url: &str) -> Option<u16> {
  let rest = url.split_once("://")?.1;
  let authority = rest.split(['/', '?', '#']).next()?;
  let (host, port) = authority.rsplit_once(':')?;
  matches!(host, "localhost" | "127.0.0.1" | "[::1]" | "0.0.0.0").then_some(())?;
  port.parse().ok()
}

/// The repository (or directory) a path belongs to: the nearest ancestor with
/// `.git`, else the path itself.
pub fn project_root(path: &str) -> PathBuf {
  let start = PathBuf::from(path);
  start
    .ancestors()
    .find(|dir| dir.join(".git").exists())
    .map(Path::to_path_buf)
    .unwrap_or(start)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn claude_working_with_spinner() {
    let screen = "stuff\n✶ Burrowing… (2m 39s · ↓ 7.0k tokens)\n───\n❯ \n───\n  ⏵⏵ auto mode on · 1 shell · esc to interrupt\n";
    let agent = parse_screen(AgentKind::Claude, "/p", screen);
    assert_eq!(agent.state, AgentState::Working);
    assert_eq!(
      agent.activity.as_deref(),
      Some("Burrowing… (2m 39s · ↓ 7.0k tokens)")
    );
  }

  #[test]
  fn claude_idle_with_recap() {
    let screen = "※ recap: On main, thumbnails are fixed. Next, commit them. (disable\n  recaps in /config)\n───\n❯ \n───\n  ⏵⏵ auto mode on\n";
    let agent = parse_screen(AgentKind::Claude, "/p", screen);
    assert_eq!(agent.state, AgentState::Idle);
    assert_eq!(
      agent.note.as_deref(),
      Some("On main, thumbnails are fixed. Next, commit them.")
    );
  }

  #[test]
  fn codex_footer_task() {
    let screen = "  Worked for 4m 18s · done 8:20 AM\n›Ask Codex to do anything\n  gpt-6 xhigh · ~/dev/artcraft · Route Wan models\n";
    let agent = parse_screen(AgentKind::Codex, "/p", screen);
    assert_eq!(agent.state, AgentState::Idle);
    assert_eq!(agent.note.as_deref(), Some("Route Wan models"));
    assert_eq!(
      agent.activity.as_deref(),
      Some("Worked for 4m 18s · done 8:20 AM")
    );
  }

  #[test]
  fn local_ports() {
    assert_eq!(local_port("http://localhost:4201/create"), Some(4201));
    assert_eq!(local_port("http://127.0.0.1:5000"), Some(5000));
    assert_eq!(local_port("https://example.com:8443/"), None);
  }
}
