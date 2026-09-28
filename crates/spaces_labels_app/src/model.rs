//! What each overlay window shows, derived from the latest Spaces snapshot.

use crate::naming::NameStore;
use crate::vision::{VisionNote, VisionStatus};
use app_context::SpaceContext;
use serde::{Deserialize, Serialize};
use spaces_sys::{AppsBySpace, Snapshot, Space, SpaceId, SpaceKind};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
  TopLeft,
  TopRight,
  BottomLeft,
  #[default]
  BottomRight,
  /// Dead center of the screen.
  Center,
  /// Dead center, at poster size.
  CenterBig,
  /// Centered horizontally, vertically centered on the top quarter line:
  /// where the eye lands mid-switch, like a website's hero section.
  Hero,
  /// Hero position at poster size.
  HeroBig,
}

impl Placement {
  pub const CORNERS: [(Placement, &'static str); 4] = [
    (Placement::TopLeft, "Top left"),
    (Placement::TopRight, "Top right"),
    (Placement::BottomLeft, "Bottom left"),
    (Placement::BottomRight, "Bottom right"),
  ];
  pub const CENTERED: [(Placement, &'static str); 4] = [
    (Placement::Center, "Center"),
    (Placement::CenterBig, "Center (big)"),
    (Placement::Hero, "Hero (upper center)"),
    (Placement::HeroBig, "Hero (upper center, big)"),
  ];

  pub fn all() -> impl Iterator<Item = (Placement, &'static str)> {
    Self::CORNERS.into_iter().chain(Self::CENTERED)
  }

  pub fn id(self) -> &'static str {
    match self {
      Placement::TopLeft => "placement:top_left",
      Placement::TopRight => "placement:top_right",
      Placement::BottomLeft => "placement:bottom_left",
      Placement::BottomRight => "placement:bottom_right",
      Placement::Center => "placement:center",
      Placement::CenterBig => "placement:center_big",
      Placement::Hero => "placement:hero",
      Placement::HeroBig => "placement:hero_big",
    }
  }

  /// Overlay window size (points). Centered placements get room for bigger
  /// type; the page centers its content inside.
  pub fn window_size(self) -> (f64, f64) {
    match self {
      Placement::CenterBig => (1200.0, 720.0),
      // Shorter than CenterBig so the window can center on the quarter line
      // without being pushed down against the menu bar.
      Placement::HeroBig => (1200.0, 520.0),
      Placement::Center | Placement::Hero => (720.0, 420.0),
      _ => (480.0, 360.0),
    }
  }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OverlayState {
  pub space_id: SpaceId,
  /// Your name for the desktop, else the model's, else `desktop`.
  pub name: String,
  /// "user", "ai" or "default".
  pub name_source: &'static str,
  /// Mission Control's name ("Desktop 3").
  pub desktop: String,
  pub ai_name: Option<String>,
  pub ai_summary: Option<String>,
  pub user_description: Option<String>,
  pub color: &'static str,
  pub apps: Vec<String>,
  pub placement: Placement,
  pub show_apps: bool,
  /// Tabs, terminals and titles for the "more" view. `None` until the first
  /// (slower) context pass has run.
  pub context: Option<SpaceContext>,
  /// Screen Recording granted, so other apps' window titles are known.
  pub titles_readable: bool,
  pub expanded: bool,
  pub vision: VisionStatus,
}

/// Everything an overlay's state is derived from.
pub struct Sources<'a> {
  pub snapshot: &'a Snapshot,
  pub apps: &'a AppsBySpace,
  pub contexts: Option<&'a HashMap<SpaceId, SpaceContext>>,
  pub titles_readable: bool,
  pub placement: Placement,
  pub show_apps: bool,
  pub vision: &'a HashMap<u32, VisionNote>,
  pub vision_status: &'a VisionStatus,
  pub names: &'a NameStore,
}

/// What to call a desktop: your name, else the model's, else Mission
/// Control's. Returns the name and where it came from.
pub fn display_name(space: &Space, names: &NameStore) -> (String, &'static str) {
  let entry = names.get(&space.uuid);
  if let Some(user) = entry.and_then(|n| n.user.as_ref()) {
    return (user.name.clone(), "user");
  }
  if let Some(ai) = entry.and_then(|n| n.ai.as_ref()) {
    return (ai.name.clone(), "ai");
  }
  (space_name(space), "default")
}

/// Fills in the vision model's latest description of each window.
pub fn with_vision(mut context: SpaceContext, vision: &HashMap<u32, VisionNote>) -> SpaceContext {
  for window in context.apps.iter_mut().flat_map(|a| a.windows.iter_mut()) {
    window.vision = vision.get(&window.id).map(|note| note.text.clone());
  }
  context
}

/// Distinct hues so neighbouring desktops never look alike. Placeholder until
/// desktops get user- or VLM-chosen names and colors.
const PALETTE: [&str; 10] = [
  "#ff6b6b", "#ffa94d", "#ffd43b", "#69db7c", "#38d9a9", "#4dabf7", "#748ffc", "#b197fc",
  "#f783ac", "#e599f7",
];

pub fn space_name(space: &Space) -> String {
  match (space.kind, space.desktop_number) {
    (SpaceKind::Desktop, Some(n)) => format!("Desktop {n}"),
    (SpaceKind::Fullscreen, _) => "Full Screen".into(),
    _ => "Space".into(),
  }
}

pub fn overlay_state(sources: &Sources, space_id: SpaceId, expanded: bool) -> Option<OverlayState> {
  let (_, space) = sources.snapshot.space(space_id)?;
  let color = PALETTE[space.desktop_number.unwrap_or(0).saturating_sub(1) % PALETTE.len()];
  let apps = sources
    .apps
    .get(&space_id)
    .map(|list| list.iter().map(|a| a.name.clone()).collect())
    .unwrap_or_default();
  let (name, name_source) = display_name(space, sources.names);
  let entry = sources.names.get(&space.uuid);
  Some(OverlayState {
    space_id,
    name,
    name_source,
    desktop: space_name(space),
    ai_name: entry.and_then(|n| n.ai.as_ref()).map(|a| a.name.clone()),
    ai_summary: entry.and_then(|n| n.ai.as_ref()).map(|a| a.summary.clone()),
    user_description: entry
      .and_then(|n| n.user.as_ref())
      .map(|u| u.description.clone())
      .filter(|d| !d.is_empty()),
    color,
    apps,
    placement: sources.placement,
    show_apps: sources.show_apps,
    context: sources.contexts.map(|all| {
      with_vision(
        all.get(&space_id).cloned().unwrap_or_default(),
        sources.vision,
      )
    }),
    titles_readable: sources.titles_readable,
    expanded,
    vision: sources.vision_status.clone(),
  })
}
