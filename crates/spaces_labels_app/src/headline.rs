//! The overlay's title and app chips, computed from facts rather than the
//! language model, so they are exact and never change on a whim:
//! * a desktop with a project is titled "<Project> (<tools>)", e.g.
//!   "ArtCraft Services (Claude)" or "ArtCraft Desktop (RustRover)";
//! * otherwise by what it is used for: "Communications (Discord)",
//!   "Fun (Reddit, YouTube)", "Research", "GitHub", …
//!
//! The model's contribution (the current task) becomes the subtitle.

use app_context::{AppContext, Entry, SpaceContext};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
  Code,
  Research,
  Github,
  Fun,
  Comms,
  Notes,
  Other,
}

impl Category {
  fn title(self) -> Option<&'static str> {
    match self {
      Category::Code => Some("Coding"),
      Category::Research => Some("Research"),
      Category::Github => Some("GitHub"),
      Category::Fun => Some("Fun"),
      Category::Comms => Some("Communications"),
      Category::Notes => Some("Notes"),
      Category::Other => None,
    }
  }
}

/// One chip under the title: an app or a coding agent.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Chip {
  pub label: String,
  pub category: Category,
  /// "claude" or "codex" for agents (the page draws their icons); apps use
  /// their macOS icon.
  pub agent: Option<&'static str>,
  /// Agents: "working", "waiting" or "idle".
  pub state: Option<&'static str>,
}

const TERMINALS: [&str; 7] = [
  "Ghostty",
  "iTerm2",
  "Terminal",
  "WezTerm",
  "kitty",
  "Alacritty",
  "Warp",
];
const EDITORS: [&str; 16] = [
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
  "Code",
  "Visual Studio Code",
  "Cursor",
  "Xcode",
  "Zed",
  "Nova",
];
const BROWSERS: [&str; 9] = [
  "Google Chrome",
  "Firefox",
  "Firefox Developer Edition",
  "Firefox Nightly",
  "Safari",
  "Arc",
  "Brave Browser",
  "Microsoft Edge",
  "Orion",
];
const COMMS: [&str; 11] = [
  "Discord",
  "Slack",
  "Messages",
  "Mail",
  "Telegram",
  "WhatsApp",
  "zoom.us",
  "Microsoft Teams",
  "Signal",
  "Spark",
  "Mimestream",
];
const NOTES: [&str; 6] = ["Obsidian", "Notes", "Notion", "Bear", "Craft", "Logseq"];
const AI_CHAT: [&str; 3] = ["ChatGPT", "Claude", "Perplexity"];

/// Sites that are for fun (or procrastination), by domain suffix.
const FUN_SITES: [&str; 16] = [
  "reddit.com",
  "youtube.com",
  "x.com",
  "twitter.com",
  "instagram.com",
  "facebook.com",
  "tiktok.com",
  "twitch.tv",
  "netflix.com",
  "news.ycombinator.com",
  "imgur.com",
  "9gag.com",
  "hulu.com",
  "espn.com",
  "bsky.app",
  "threads.net",
];
const GITHUB_SITES: [&str; 3] = ["github.com", "gitlab.com", "bitbucket.org"];
const COMMS_SITES: [&str; 6] = [
  "mail.google.com",
  "calendar.google.com",
  "outlook.live.com",
  "outlook.office.com",
  "slack.com",
  "discord.com",
];

fn site_category(host: &str) -> Category {
  let on = |sites: &[&str]| {
    sites
      .iter()
      .any(|s| host == *s || host.ends_with(&format!(".{s}")))
  };
  if on(&GITHUB_SITES) {
    Category::Github
  } else if on(&FUN_SITES) {
    Category::Fun
  } else if on(&COMMS_SITES) {
    Category::Comms
  } else if host.starts_with("localhost") || host.starts_with("127.0.0.1") {
    Category::Code
  } else {
    Category::Research
  }
}

/// "news.ycombinator.com" -> "Hacker News", "youtube.com" -> "YouTube".
fn site_name(host: &str) -> String {
  let known = [
    ("reddit.com", "Reddit"),
    ("youtube.com", "YouTube"),
    ("news.ycombinator.com", "Hacker News"),
    ("x.com", "X"),
    ("twitter.com", "X"),
    ("github.com", "GitHub"),
    ("mail.google.com", "Gmail"),
    ("calendar.google.com", "Calendar"),
    ("twitch.tv", "Twitch"),
    ("chatgpt.com", "ChatGPT"),
    ("claude.ai", "Claude"),
    ("google.com", "Google"),
    ("stackoverflow.com", "Stack Overflow"),
    ("wikipedia.org", "Wikipedia"),
    ("docs.rs", "docs.rs"),
    ("huggingface.co", "Hugging Face"),
    ("linkedin.com", "LinkedIn"),
  ];
  if let Some((_, name)) = known
    .iter()
    .find(|(h, _)| host == *h || host.ends_with(&format!(".{h}")))
  {
    return name.to_string();
  }
  let label = host.split('.').rev().nth(1).unwrap_or(host);
  let mut chars = label.chars();
  chars
    .next()
    .map(|c| c.to_uppercase().chain(chars).collect())
    .unwrap_or_default()
}

/// Category weights from a browser's tabs: active tabs count most.
fn browser_weights(app: &AppContext) -> HashMap<Category, (f32, Vec<String>)> {
  let mut weights: HashMap<Category, (f32, Vec<String>)> = HashMap::new();
  for window in &app.windows {
    for tab in &window.tabs {
      let Some(host) = &tab.detail else { continue };
      let category = site_category(host);
      let entry = weights.entry(category).or_default();
      entry.0 += if tab.active { 3.0 } else { 0.5 };
      let name = site_name(host);
      if tab.active && !entry.1.contains(&name) {
        entry.1.push(name);
      }
    }
  }
  weights
}

pub fn app_category(app: &AppContext) -> Category {
  let name = app.name.as_str();
  if TERMINALS.contains(&name) || EDITORS.contains(&name) {
    Category::Code
  } else if COMMS.contains(&name) {
    Category::Comms
  } else if NOTES.contains(&name) {
    Category::Notes
  } else if AI_CHAT.contains(&name) {
    Category::Research
  } else if BROWSERS.contains(&name) {
    browser_weights(app)
      .into_iter()
      .max_by(|a, b| a.1 .0.total_cmp(&b.1 .0))
      .map(|(c, _)| c)
      .unwrap_or(Category::Research)
  } else {
    Category::Other
  }
}

/// "artcraft-services" -> "Artcraft Services", unless configured otherwise.
pub fn project_display(dir: &str, aliases: &HashMap<String, String>) -> String {
  if let Some(alias) = aliases.get(dir) {
    return alias.clone();
  }
  dir
    .split(['-', '_', ' '])
    .filter(|w| !w.is_empty())
    .map(|w| {
      let mut chars = w.chars();
      chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect::<String>())
        .unwrap_or_default()
    })
    .collect::<Vec<_>>()
    .join(" ")
}

fn agents(context: &SpaceContext) -> Vec<(&'static str, &'static str)> {
  fn walk(entry: &Entry, out: &mut Vec<(&'static str, &'static str)>) {
    if let Some(status) = &entry.status {
      let agent = if status.starts_with("Claude") {
        Some("claude")
      } else if status.starts_with("Codex") {
        Some("codex")
      } else {
        None
      };
      let state = if status.contains("working") {
        "working"
      } else if status.contains("waiting") {
        "waiting"
      } else {
        "idle"
      };
      if let Some(agent) = agent {
        match out.iter_mut().find(|(a, _)| *a == agent) {
          // The busiest state wins when several sessions share an agent.
          Some(existing) if state == "working" || (state == "waiting" && existing.1 == "idle") => {
            existing.1 = state
          }
          Some(_) => {}
          None => out.push((agent, state)),
        }
      }
    }
    for child in &entry.children {
      walk(child, out);
    }
  }
  let mut out = Vec::new();
  for app in &context.apps {
    for window in &app.windows {
      for tab in &window.tabs {
        walk(tab, &mut out);
      }
    }
  }
  out
}

/// The deterministic title, or `None` when nothing identifies the desktop.
pub fn title(context: &SpaceContext, aliases: &HashMap<String, String>) -> Option<String> {
  if let Some(project) = context.projects.first() {
    let mut tools: Vec<String> = Vec::new();
    for signal in &project.signals {
      let tool = signal.split_whitespace().next().unwrap_or_default();
      let is_tool = matches!(tool, "Claude" | "Codex") || EDITORS.contains(&signal.as_str());
      let tool = if EDITORS.contains(&signal.as_str()) {
        signal.as_str()
      } else {
        tool
      };
      if is_tool && !tools.iter().any(|t| t == tool) {
        tools.push(tool.to_string());
      }
    }
    let name = project_display(&project.name, aliases);
    return Some(if tools.is_empty() {
      name
    } else {
      format!("{name} ({})", tools.join(", "))
    });
  }
  let mut weights: HashMap<Category, (f32, Vec<String>)> = HashMap::new();
  for app in &context.apps {
    if BROWSERS.contains(&app.name.as_str()) {
      for (category, (weight, sites)) in browser_weights(app) {
        let entry = weights.entry(category).or_default();
        entry.0 += weight;
        entry.1.extend(
          sites
            .into_iter()
            .filter(|s| !entry.1.contains(s))
            .collect::<Vec<_>>(),
        );
      }
    } else {
      let category = app_category(app);
      let entry = weights.entry(category).or_default();
      // A dedicated app (Discord) outweighs any single browser tab (3).
      entry.0 += 4.0;
      if category != Category::Research || AI_CHAT.contains(&app.name.as_str()) {
        entry.1.push(app.name.clone());
      }
    }
  }
  let (category, (_, names)) = weights
    .into_iter()
    .filter(|(c, _)| c.title().is_some())
    .max_by(|a, b| a.1 .0.total_cmp(&b.1 .0))?;
  let label = category.title()?;
  let names: Vec<String> = names.into_iter().filter(|n| n != label).take(2).collect();
  Some(if names.is_empty() {
    label.to_string()
  } else {
    format!("{label} ({})", names.join(", "))
  })
}

/// Agents first (with their state), then apps front-most first.
pub fn chips(context: &SpaceContext) -> Vec<Chip> {
  let mut chips: Vec<Chip> = agents(context)
    .into_iter()
    .map(|(agent, state)| Chip {
      label: if agent == "claude" { "Claude" } else { "Codex" }.to_string(),
      category: Category::Code,
      agent: Some(agent),
      state: Some(state),
    })
    .collect();
  chips.extend(context.apps.iter().map(|app| Chip {
    label: app.name.clone(),
    category: app_category(app),
    agent: None,
    state: None,
  }));
  chips
}

#[cfg(test)]
mod tests {
  use super::*;
  use app_context::{Project, WindowContext};

  fn app(name: &str, tabs: &[(&str, bool)]) -> AppContext {
    AppContext {
      name: name.into(),
      windows: vec![WindowContext {
        tabs: tabs
          .iter()
          .map(|(host, active)| Entry {
            title: "t".into(),
            detail: Some(host.to_string()),
            active: *active,
            ..Default::default()
          })
          .collect(),
        ..Default::default()
      }],
    }
  }

  #[test]
  fn project_titles_name_their_tools() {
    let aliases = HashMap::from([("artcraft".to_string(), "ArtCraft Desktop".to_string())]);
    let mut context = SpaceContext {
      projects: vec![Project::new("artcraft", &["RustRover"])],
      ..Default::default()
    };
    assert_eq!(
      title(&context, &aliases).as_deref(),
      Some("ArtCraft Desktop (RustRover)")
    );
    context.projects = vec![Project::new(
      "artcraft-services",
      &["Claude idle", "terminal", "dev site :4201"],
    )];
    assert_eq!(
      title(&context, &aliases).as_deref(),
      Some("Artcraft Services (Claude)")
    );
  }

  #[test]
  fn desktops_without_projects_are_titled_by_use() {
    let aliases = HashMap::new();
    let context = SpaceContext {
      apps: vec![app("Discord", &[]), app("Firefox", &[("reddit.com", true)])],
      ..Default::default()
    };
    assert_eq!(
      title(&context, &aliases).as_deref(),
      Some("Communications (Discord)")
    );
    let context = SpaceContext {
      apps: vec![app(
        "Firefox",
        &[
          ("reddit.com", true),
          ("youtube.com", false),
          ("docs.rs", false),
        ],
      )],
      ..Default::default()
    };
    assert_eq!(title(&context, &aliases).as_deref(), Some("Fun (Reddit)"));
  }
}
