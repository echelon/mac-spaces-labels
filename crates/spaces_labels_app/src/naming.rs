//! Names each desktop from what is on it, with the local model.
//!
//! A desktop's context (projects, coding agents, windows, tabs, screenshot
//! descriptions) is condensed into a short, deterministic text digest. The
//! model turns it into `{"name", "summary"}` (JSON-schema constrained). Names
//! are cached aggressively and are sticky:
//! * a desktop with no name is named as soon as the model is up;
//! * a changed set of projects renames it, at most every [`RETRY_AFTER`];
//! * any other change renames it only after [`RENAME_AFTER`].
//!
//! Names you type win over the model's, and each one is appended with the
//! exact model input to `feedback.jsonl`, as eval data.

use crate::engine::Engine;
use crate::model::with_vision;
use crate::vision::http;
use app_context::SpaceContext;
use serde::{Deserialize, Serialize};
use spaces_sys::{Space, SpaceId};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const RENAME_AFTER: Duration = Duration::from_secs(600);
const RETRY_AFTER: Duration = Duration::from_secs(30);
pub const NAMES_FILE: &str = "names.json";
pub const FEEDBACK_FILE: &str = "feedback.jsonl";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SpaceNames {
  pub ai: Option<AiName>,
  pub user: Option<UserName>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AiName {
  pub name: String,
  /// The task (or, without a project, the topic) alone: the subtitle.
  #[serde(default)]
  pub task: Option<String>,
  pub summary: String,
  pub model: String,
  pub at_unix: u64,
  pub seconds: f32,
  /// Hash of the digest it was made from, and of its project set.
  pub input_hash: u64,
  pub projects_hash: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UserName {
  pub name: String,
  pub description: String,
  pub at_unix: u64,
}

/// Space UUID (stable across restarts and reordering) -> names.
pub type NameStore = HashMap<String, SpaceNames>;

pub fn load(dir: &Path) -> NameStore {
  std::fs::read(dir.join(NAMES_FILE))
    .ok()
    .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    .unwrap_or_default()
}

pub fn save(dir: &Path, store: &NameStore) {
  if let Ok(json) = serde_json::to_vec_pretty(store) {
    let _ = std::fs::write(dir.join(NAMES_FILE), json);
  }
}

pub fn now_unix() -> u64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_secs())
    .unwrap_or(0)
}

fn hash(text: &str) -> u64 {
  let mut h = DefaultHasher::new();
  text.hash(&mut h);
  h.finish()
}

fn cut(text: &str, max: usize) -> String {
  let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
  if text.chars().count() <= max {
    return text;
  }
  let mut out: String = text.chars().take(max - 1).collect();
  out.push('…');
  out
}

/// Agent status without its live timer ("Claude working · Burrowing… (2m)"
/// -> "Claude working"), so the digest only changes when something real does.
fn stable_status(status: &str) -> &str {
  status.split(" · ").next().unwrap_or(status)
}

/// The model input for one desktop: projects first, then each window with
/// its active tab first, agents and their notes, and screenshot descriptions.
pub fn digest(context: &SpaceContext) -> String {
  let mut lines = Vec::new();
  for project in &context.projects {
    lines.push(format!(
      "Project: {} ({})",
      project.name,
      project.signals.join(", ")
    ));
  }
  for app in &context.apps {
    for window in &app.windows {
      let mut head = format!("- {}", app.name);
      if let Some(title) = &window.title {
        head.push_str(&format!(" \"{}\"", cut(title, 70)));
      }
      lines.push(head);
      let mut tabs: Vec<_> = window.tabs.iter().collect();
      tabs.sort_by_key(|t| !t.active);
      for tab in tabs.iter().take(8) {
        let mut line = format!("    tab: {}", cut(&tab.title, 70));
        if let Some(detail) = &tab.detail {
          line.push_str(&format!(" [{detail}]"));
        }
        if let Some(status) = &tab.status {
          line.push_str(&format!(" — {}", stable_status(status)));
        }
        lines.push(line);
        for child in tab.children.iter().take(6) {
          let mut line = format!(
            "      tmux {} in {}",
            cut(&child.title, 40),
            child.detail.as_deref().unwrap_or("?")
          );
          if let Some(status) = &child.status {
            line.push_str(&format!(" — {}", stable_status(status)));
          }
          lines.push(line);
          if let Some(note) = &child.note {
            lines.push(format!("        agent note: {}", cut(note, 160)));
          }
        }
      }
      if window.tabs.len() > 8 {
        lines.push(format!("    (+{} more tabs)", window.tabs.len() - 8));
      }
      // Failed screenshots ("(no description: …)") are not evidence.
      if let Some(vision) = window.vision.as_ref().filter(|v| !v.starts_with('(')) {
        lines.push(format!("    screenshot: {}", cut(vision, 160)));
      }
    }
  }
  lines.join("\n")
}

fn projects_hash(context: &SpaceContext) -> u64 {
  let mut names: Vec<&str> = context.projects.iter().map(|p| p.name.as_str()).collect();
  names.sort();
  hash(&names.join("\n"))
}

pub struct Job {
  pub space: SpaceId,
  /// The top project, which anchors the name.
  pub project: Option<String>,
  pub uuid: String,
  pub digest: String,
  pub input_hash: u64,
  pub projects_hash: u64,
}

/// The desktop most in need of a (new) name, showing desktops first.
pub fn next_job(engine: &Engine, attempts: &HashMap<SpaceId, Instant>) -> Option<Job> {
  let model = engine.model.lock().unwrap();
  let contexts = model.contexts.as_ref()?;
  let showing: Vec<SpaceId> = model.snapshot.current_spaces().collect();
  let mut spaces: Vec<&Space> = model
    .snapshot
    .displays
    .iter()
    .flat_map(|d| &d.spaces)
    .collect();
  spaces.sort_by_key(|s| !showing.contains(&s.id));
  spaces.into_iter().find_map(|space| {
    let context = with_vision(
      contexts.get(&space.id).cloned().unwrap_or_default(),
      &model.vision,
    );
    let digest = digest(&context);
    if digest.is_empty() {
      return None;
    }
    let (input_hash, projects_hash) = (hash(&digest), projects_hash(&context));
    let tried = attempts.get(&space.id).map(Instant::elapsed);
    let due = match model.names.get(&space.uuid).and_then(|n| n.ai.as_ref()) {
      None => tried.is_none_or(|t| t > RETRY_AFTER),
      Some(ai) if ai.projects_hash != projects_hash => tried.is_none_or(|t| t > RETRY_AFTER),
      Some(ai) if ai.input_hash != input_hash => {
        now_unix().saturating_sub(ai.at_unix) > RENAME_AFTER.as_secs()
          && tried.is_none_or(|t| t > RENAME_AFTER)
      }
      Some(_) => false,
    };
    due.then(|| Job {
      space: space.id,
      project: context.projects.first().map(|p| p.name.clone()),
      uuid: space.uuid.clone(),
      digest,
      input_hash,
      projects_hash,
    })
  })
}

/// With a known project (from agents, terminals, dev sites or IDE titles),
/// the project is the name's anchor and the model only names the task: small
/// models otherwise drift to app names ("Ghostty: …") or to the agent's latest
/// step.
const TASK_PROMPT: &str = "You help someone with ADHD remember what each macOS desktop is for. The desktop's \
main project is {project}. From what is open, reply with JSON: {\"task\": the overall goal on this desktop in \
1-3 words, Title Case, not the latest step and not the project or app name; \"summary\": one short sentence on \
what is happening}.";

const TOPIC_PROMPT: &str = "You help someone with ADHD remember what each macOS desktop is for. Nothing here \
belongs to a code project. From what is open, reply with JSON: {\"name\": the topic in 2-4 words, Title Case, \
e.g. \"Tax Paperwork\" or \"Trip Planning\", never an app name or generic like \"Browsing\"; \"summary\": one \
short sentence}.";

/// Names a desktop: "<project>: <task>", or a topic when there is no
/// project. ~0.5–1.5 s on Apple silicon.
pub fn name(
  port: u16,
  digest: &str,
  project: Option<&str>,
) -> Result<(String, String, String), String> {
  let (system, key) = match project {
    Some(project) => (TASK_PROMPT.replace("{project}", project), "task"),
    None => (TOPIC_PROMPT.to_string(), "name"),
  };
  let body = serde_json::json!({
    "messages": [
      {"role": "system", "content": system},
      {"role": "user", "content": digest},
    ],
    "max_tokens": 90,
    "temperature": 0,
    "response_format": {"type": "json_schema", "json_schema": {"schema": {
      "type": "object",
      "properties": {key: {"type": "string"}, "summary": {"type": "string"}},
      "required": [key, "summary"],
    }}},
  })
  .to_string();
  let (status, reply) = http(
    port,
    "POST",
    "/v1/chat/completions",
    Some(&body),
    Duration::from_secs(60),
  )?;
  if status != 200 {
    return Err(format!("model server returned {status}"));
  }
  let reply: serde_json::Value = serde_json::from_slice(&reply).map_err(|e| e.to_string())?;
  let content = reply["choices"][0]["message"]["content"]
    .as_str()
    .ok_or("empty reply")?;
  let named: serde_json::Value = serde_json::from_str(content).map_err(|e| e.to_string())?;
  let label = cut(named[key].as_str().unwrap_or_default(), 32);
  let summary = cut(named["summary"].as_str().unwrap_or_default(), 200);
  if label.is_empty() {
    return Err("no name".into());
  }
  let name = match project {
    Some(project) => format!("{project}: {label}"),
    None => label.clone(),
  };
  Ok((name, label, summary))
}

/// Appends a rename (with what the model saw and said) to the eval log.
pub fn record_feedback(dir: &Path, record: &serde_json::Value) {
  let Ok(mut file) = std::fs::OpenOptions::new()
    .create(true)
    .append(true)
    .open(dir.join(FEEDBACK_FILE))
  else {
    return;
  };
  if let Ok(line) = serde_json::to_string(record) {
    let _ = writeln!(file, "{line}");
  }
}
