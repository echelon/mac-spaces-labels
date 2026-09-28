use crate::cf::{self, Owned};
use crate::ffi::{self, CgRect};

/// Global bounds (points, top-left origin) of the display with this Spaces
/// identifier. `"Main"` is what SkyLight reports when all displays share one
/// set of Spaces.
pub fn display_bounds(uuid: &str) -> Option<CgRect> {
  unsafe {
    if uuid == "Main" {
      return Some(ffi::CGDisplayBounds(ffi::CGMainDisplayID()));
    }
    let mut ids = [0u32; 32];
    let mut count = 0u32;
    if ffi::CGGetActiveDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) != 0 {
      return None;
    }
    ids[..count as usize]
      .iter()
      .copied()
      .find(|&id| display_uuid(id).is_some_and(|u| u.eq_ignore_ascii_case(uuid)))
      .map(|id| ffi::CGDisplayBounds(id))
  }
}

unsafe fn display_uuid(display: u32) -> Option<String> {
  let uuid = Owned::new(ffi::CGDisplayCreateUUIDFromDisplayID(display))?;
  let text = Owned::new(ffi::CFUUIDCreateString(std::ptr::null(), uuid.as_ptr()) as _)?;
  cf::string(text.as_ptr())
}
