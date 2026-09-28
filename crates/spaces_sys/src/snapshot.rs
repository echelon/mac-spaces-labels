use crate::cf::{self, Key, Owned};
use crate::ffi::{self, ConnectionId, SpaceId};
use serde::Serialize;
use std::sync::OnceLock;

/// Every display's Spaces, in Mission Control order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Snapshot {
  pub displays: Vec<Display>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Display {
  /// A display UUID, or `"Main"` when "Displays have separate Spaces" is off.
  pub uuid: String,
  pub current_space: SpaceId,
  pub spaces: Vec<Space>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Space {
  /// The `ManagedSpaceID`. Stable while the Space exists, even when reordered.
  pub id: SpaceId,
  pub uuid: String,
  pub kind: SpaceKind,
  /// 1-based number among the ordinary desktops on this display, as Mission
  /// Control shows it ("Desktop 3"). `None` for full-screen app Spaces.
  pub desktop_number: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpaceKind {
  Desktop,
  Fullscreen,
  Other,
}

impl Snapshot {
  pub fn space(&self, id: SpaceId) -> Option<(&Display, &Space)> {
    self
      .displays
      .iter()
      .find_map(|d| d.spaces.iter().find(|s| s.id == id).map(|s| (d, s)))
  }

  pub fn current_spaces(&self) -> impl Iterator<Item = SpaceId> + '_ {
    self.displays.iter().map(|d| d.current_space)
  }
}

struct Keys {
  display_identifier: Key,
  current_space: Key,
  spaces: Key,
  managed_space_id: Key,
  uuid: Key,
  kind: Key,
}

fn keys() -> &'static Keys {
  static KEYS: OnceLock<Keys> = OnceLock::new();
  KEYS.get_or_init(|| Keys {
    display_identifier: Key::new("Display Identifier"),
    current_space: Key::new("Current Space"),
    spaces: Key::new("Spaces"),
    managed_space_id: Key::new("ManagedSpaceID"),
    uuid: Key::new("uuid"),
    kind: Key::new("type"),
  })
}

pub(crate) fn read(cid: ConnectionId) -> Snapshot {
  let k = keys();
  unsafe {
    let Some(array) = Owned::new(ffi::CGSCopyManagedDisplaySpaces(cid) as _) else {
      return Snapshot::default();
    };
    let displays = cf::array_items(array.as_ptr())
      .map(|display| {
        let uuid = cf::dict_get(display, &k.display_identifier)
          .and_then(|v| cf::string(v))
          .unwrap_or_default();
        let current_space = cf::dict_get(display, &k.current_space)
          .and_then(|s| cf::dict_get(s, &k.managed_space_id))
          .and_then(|v| cf::number_i64(v))
          .unwrap_or_default() as SpaceId;
        let mut desktops = 0;
        let spaces = cf::dict_get(display, &k.spaces)
          .map(|spaces| {
            cf::array_items(spaces)
              .filter_map(|space| {
                let id = cf::number_i64(cf::dict_get(space, &k.managed_space_id)?)? as SpaceId;
                let kind = match cf::dict_get(space, &k.kind).and_then(|v| cf::number_i64(v)) {
                  Some(0) => SpaceKind::Desktop,
                  Some(4) => SpaceKind::Fullscreen,
                  _ => SpaceKind::Other,
                };
                let desktop_number = (kind == SpaceKind::Desktop).then(|| {
                  desktops += 1;
                  desktops
                });
                let uuid = cf::dict_get(space, &k.uuid)
                  .and_then(|v| cf::string(v))
                  .unwrap_or_default();
                Some(Space {
                  id,
                  uuid,
                  kind,
                  desktop_number,
                })
              })
              .collect()
          })
          .unwrap_or_default();
        Display {
          uuid,
          current_space,
          spaces,
        }
      })
      .collect();
    Snapshot { displays }
  }
}
