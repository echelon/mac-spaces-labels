//! What each overlay window shows, derived from the latest Spaces snapshot.

use serde::{Deserialize, Serialize};
use spaces_sys::{AppsBySpace, Snapshot, Space, SpaceId, SpaceKind};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Corner {
  TopLeft,
  TopRight,
  BottomLeft,
  #[default]
  BottomRight,
}

impl Corner {
  pub const ALL: [(Corner, &'static str); 4] = [
    (Corner::TopLeft, "Top left"),
    (Corner::TopRight, "Top right"),
    (Corner::BottomLeft, "Bottom left"),
    (Corner::BottomRight, "Bottom right"),
  ];

  pub fn id(self) -> &'static str {
    match self {
      Corner::TopLeft => "corner:top_left",
      Corner::TopRight => "corner:top_right",
      Corner::BottomLeft => "corner:bottom_left",
      Corner::BottomRight => "corner:bottom_right",
    }
  }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OverlayState {
  pub space_id: SpaceId,
  pub name: String,
  pub color: &'static str,
  pub apps: Vec<String>,
  pub corner: Corner,
  pub show_apps: bool,
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

pub fn overlay_state(
  snapshot: &Snapshot,
  apps: &AppsBySpace,
  space_id: SpaceId,
  corner: Corner,
  show_apps: bool,
) -> Option<OverlayState> {
  let (_, space) = snapshot.space(space_id)?;
  let color = PALETTE[space.desktop_number.unwrap_or(0).saturating_sub(1) % PALETTE.len()];
  let apps = apps
    .get(&space_id)
    .map(|list| list.iter().map(|a| a.name.clone()).collect())
    .unwrap_or_default();
  Some(OverlayState {
    space_id,
    name: space_name(space),
    color,
    apps,
    corner,
    show_apps,
  })
}
