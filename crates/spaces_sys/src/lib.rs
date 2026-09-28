//! macOS Spaces ("virtual desktops") for Rust, without any user permissions.
//!
//! * [`Spaces::snapshot`] lists every display's Spaces in Mission Control order
//!   and which one is showing. It is a single WindowServer round trip.
//! * [`Spaces::active_space`] is the cheapest possible "where am I" query and is
//!   safe to call from a tight poll loop.
//! * [`Spaces::apps_by_space`] maps each Space to the apps that own normal
//!   windows on it, front-most first.
//!
//! All data comes from the private SkyLight connection API; see [`ffi`].

#![cfg(target_os = "macos")]

mod cf;
mod displays;
pub mod ffi;
mod snapshot;
mod windows;

pub use displays::display_bounds;
pub use ffi::{CgRect, SpaceId};
pub use snapshot::{Display, Snapshot, Space, SpaceKind};
pub use windows::{AppOnSpace, AppsBySpace};

/// A handle to this process's WindowServer connection. Cheap and `Copy`; the
/// connection itself lives for the life of the process.
#[derive(Clone, Copy, Debug)]
pub struct Spaces {
  cid: ffi::ConnectionId,
}

impl Spaces {
  pub fn connect() -> Self {
    Self {
      cid: unsafe { ffi::CGSMainConnectionID() },
    }
  }

  pub fn connection_id(self) -> ffi::ConnectionId {
    self.cid
  }

  /// The Space showing on the display that owns the menu bar (the one with
  /// keyboard focus). Takes a few microseconds.
  pub fn active_space(self) -> SpaceId {
    unsafe { ffi::CGSGetActiveSpace(self.cid) }
  }

  pub fn snapshot(self) -> Snapshot {
    snapshot::read(self.cid)
  }

  /// The Spaces each of the given windows belongs to (sticky windows belong to
  /// several).
  pub fn spaces_for_window(self, window_id: u32) -> Vec<SpaceId> {
    windows::spaces_for_window(self.cid, window_id)
  }

  /// Moves one of our own windows onto a Space that is not showing. Returns
  /// whether SkyLight honoured it (verified by reading the window back).
  pub fn move_window_to_space(self, window_id: u32, space: SpaceId) -> bool {
    let ids = cf::new_number_array(&[window_id as i64]);
    unsafe { ffi::CGSMoveWindowsToManagedSpace(self.cid, ids.as_ptr() as _, space) };
    self.spaces_for_window(window_id) == [space]
  }

  pub fn describe_windows(self, pid: i32) -> Vec<String> {
    windows::describe_windows(self.cid, pid)
  }

  /// Apps with normal windows, grouped by Space. `exclude_pid` hides our own
  /// overlay windows.
  pub fn apps_by_space(self, exclude_pid: i32) -> AppsBySpace {
    windows::apps_by_space(self.cid, exclude_pid)
  }
}
