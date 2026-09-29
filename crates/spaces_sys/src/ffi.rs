//! Raw declarations for the private SkyLight ("CGS") calls this crate uses.
//!
//! These are undocumented but have been stable for a decade and back tools such
//! as yabai, Amethyst, and Hammerspoon. None of them need Accessibility or
//! Screen Recording permission. CoreGraphics re-exports the CGS names from
//! SkyLight, so linking the public framework is enough.

use core_foundation::array::CFArrayRef;
use core_foundation::string::CFStringRef;
use std::ffi::c_void;

pub type ConnectionId = i32;
pub type SpaceId = u64;
pub type CgError = i32;

/// `CGSCopySpacesForWindows` mask: current + other + user spaces.
pub const ALL_SPACES_MASK: i32 = 0x7;

pub type NotifyProc = extern "C" fn(event: u32, data: *mut c_void, len: usize, user: *mut c_void);

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
  pub fn CGSMainConnectionID() -> ConnectionId;
  pub fn CGSGetActiveSpace(cid: ConnectionId) -> SpaceId;
  pub fn CGSManagedDisplayGetCurrentSpace(cid: ConnectionId, display: CFStringRef) -> SpaceId;
  pub fn CGSCopyManagedDisplaySpaces(cid: ConnectionId) -> CFArrayRef;
  pub fn CGSCopySpacesForWindows(cid: ConnectionId, mask: i32, windows: CFArrayRef) -> CFArrayRef;
  pub fn CGSGetWindowTags(cid: ConnectionId, window: u32, tags: *mut u32, bits: i32) -> CgError;
  pub fn CGSRegisterNotifyProc(proc_: NotifyProc, event: u32, user: *mut c_void) -> CgError;

  // Public CoreGraphics.
  pub fn CGWindowListCopyWindowInfo(options: u32, relative_to: u32) -> CFArrayRef;
}

/// `kCGWindowListOptionAll` and `kCGWindowListExcludeDesktopElements`.
pub const WINDOW_LIST_ALL: u32 = 0;
pub const WINDOW_LIST_EXCLUDE_DESKTOP: u32 = 1 << 4;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize)]
pub struct CgRect {
  pub x: f64,
  pub y: f64,
  pub width: f64,
  pub height: f64,
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
  pub fn CGSMoveWindowsToManagedSpace(cid: ConnectionId, windows: CFArrayRef, space: SpaceId);
  pub fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> CgError;
  pub fn CGMainDisplayID() -> u32;
  pub fn CGDisplayBounds(display: u32) -> CgRect;
  pub fn CGPreflightScreenCaptureAccess() -> bool;
  pub fn CGEventCreate(source: *const c_void) -> *mut c_void;
  pub fn CGEventSourceFlagsState(state: i32) -> u64;
  pub fn CGEventGetLocation(event: *const c_void) -> CgPoint;
  pub fn CGRequestScreenCaptureAccess() -> bool;
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CgPoint {
  pub x: f64,
  pub y: f64,
}

#[link(name = "ColorSync", kind = "framework")]
extern "C" {
  pub fn CGDisplayCreateUUIDFromDisplayID(display: u32) -> *const c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
  pub fn CFUUIDCreateString(allocator: *const c_void, uuid: *const c_void) -> CFStringRef;
}
