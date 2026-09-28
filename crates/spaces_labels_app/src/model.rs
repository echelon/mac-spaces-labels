//! What each overlay window shows, derived from the latest Spaces snapshot.

use serde::{Deserialize, Serialize};
use spaces_sys::{AppsBySpace, Snapshot, Space, SpaceId, SpaceKind};

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

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OverlayState {
  pub space_id: SpaceId,
  pub name: String,
  pub color: &'static str,
  pub apps: Vec<String>,
  pub placement: Placement,
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
  placement: Placement,
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
    placement,
    show_apps,
  })
}
