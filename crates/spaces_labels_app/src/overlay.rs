//! One transparent, click-through overlay window per Space.
//!
//! A window that is *not* `CanJoinAllSpaces` belongs to exactly one Space and
//! slides in and out with it during the switch animation. Giving every Space
//! its own pre-rendered window means the right label is already on screen when
//! the Space appears: nothing has to be detected or repainted, so the visible
//! latency is zero. A single all-Spaces window would instead show the previous
//! label until the switch was noticed and the webview repainted.

use crate::model::Placement;
use spaces_sys::{SpaceId, Spaces};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Duration;
use tauri::{
  AppHandle, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};

const MARGIN: f64 = 12.0;

pub struct Created {
  pub label: String,
  pub window_number: u32,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Creates an overlay pinned to `space`, whether or not it is showing.
/// Returns `None` if the window could not be built or placed.
/// Must not be called from the main thread (it waits for it).
pub fn create(
  app: &AppHandle,
  spaces: Spaces,
  display: &str,
  placement: Placement,
  space: SpaceId,
) -> Option<Created> {
  // Labels are never reused so a closing window cannot collide with its
  // replacement.
  let label = format!("space-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed));
  let (position, (width, height)) = frame_for(app, display, placement)?;
  let window = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("index.html".into()))
    .title("Spaces Labels")
    .inner_size(width, height)
    .position(position.x, position.y)
    .decorations(false)
    .transparent(true)
    .shadow(false)
    .resizable(false)
    .focused(false)
    .visible(false)
    .skip_taskbar(true)
    .build()
    .ok()?;
  disable_occlusion_throttling(&window);

  let (tx, rx) = mpsc::sync_channel(1);
  let native = window.clone();
  let _ = app.run_on_main_thread(move || {
    let _ = tx.send(unsafe { configure_and_show(&native, spaces, space) });
  });
  match rx.recv_timeout(Duration::from_secs(2)).ok().flatten() {
    Some(window_number) => Some(Created {
      label,
      window_number,
    }),
    None => {
      let _ = window.destroy();
      None
    }
  }
}

pub fn reposition(app: &AppHandle, label: &str, display: &str, placement: Placement) {
  if let (Some(window), Some((position, (width, height)))) = (
    app.get_webview_window(label),
    frame_for(app, display, placement),
  ) {
    let _ = window.set_size(LogicalSize::new(width, height));
    let _ = window.set_position(position);
  }
}

pub fn close(app: &AppHandle, label: &str) {
  if let Some(window) = app.get_webview_window(label) {
    let _ = window.destroy();
  }
}

/// The overlay's top-left corner and size inside the display's usable area
/// (below the menu bar, beside the Dock).
fn frame_for(
  app: &AppHandle,
  display: &str,
  placement: Placement,
) -> Option<(LogicalPosition<f64>, (f64, f64))> {
  let bounds = spaces_sys::display_bounds(display)?;
  let monitors = app.available_monitors().unwrap_or_default();
  let monitor = monitors.iter().find(|m| {
    let origin = m.position().to_logical::<f64>(m.scale_factor());
    (origin.x - bounds.x).abs() < 2.0 && (origin.y - bounds.y).abs() < 2.0
  });
  let (x, y, width, height) = match monitor {
    Some(m) => {
      let area = m.work_area();
      let origin = area.position.to_logical::<f64>(m.scale_factor());
      let size = area.size.to_logical::<f64>(m.scale_factor());
      (origin.x, origin.y, size.width, size.height)
    }
    None => (
      bounds.x,
      bounds.y + 24.0,
      bounds.width,
      bounds.height - 24.0,
    ),
  };
  // Never larger than the usable area (small or scaled displays).
  let (w, h) = placement.window_size();
  let (w, h) = (w.min(width - 2.0 * MARGIN), h.min(height - 2.0 * MARGIN));
  let left = x + MARGIN;
  let right = x + width - w - MARGIN;
  let top = y + MARGIN;
  let bottom = y + height - h - MARGIN;
  let center_x = x + (width - w) / 2.0;
  let center_y = y + (height - h) / 2.0;
  let position = match placement {
    Placement::TopLeft => LogicalPosition::new(left, top),
    Placement::TopRight => LogicalPosition::new(right, top),
    Placement::BottomLeft => LogicalPosition::new(left, bottom),
    Placement::BottomRight => LogicalPosition::new(right, bottom),
    Placement::Center | Placement::CenterBig => LogicalPosition::new(center_x, center_y),
    Placement::Hero | Placement::HeroBig => {
      LogicalPosition::new(center_x, (y + height / 4.0 - h / 2.0).max(top))
    }
  };
  Some((position, (w, h)))
}

/// WebKit stops rendering webviews in occluded windows. Our overlays spend
/// most of their life on hidden Spaces, and must already show fresh content
/// the instant their Space slides in, so opt out (private but long-standing
/// WKWebView SPI; skipped if it ever disappears).
fn disable_occlusion_throttling(window: &WebviewWindow) {
  let _ = window.with_webview(|webview| unsafe {
    use objc2::runtime::{AnyObject, Bool};
    use objc2::{msg_send, sel};
    let wk: &AnyObject = &*(webview.inner() as *const AnyObject);
    let selector = sel!(_setWindowOcclusionDetectionEnabled:);
    let supported: Bool = msg_send![wk, respondsToSelector: selector];
    if supported.as_bool() {
      let _: () = msg_send![wk, _setWindowOcclusionDetectionEnabled: Bool::NO];
    }
  });
}

/// Main thread only. Returns the WindowServer window number once the window
/// is on the intended Space.
///
/// Placement is always an explicit SkyLight move, even for the showing Space:
/// a `Stationary` window ordered in the ordinary way is not assigned to any
/// Space, and an explicit target also cannot be raced by a Space switch.
unsafe fn configure_and_show(
  window: &WebviewWindow,
  spaces: Spaces,
  space: SpaceId,
) -> Option<u32> {
  use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior as B};
  let native = &*(window.ns_window().ok()? as *mut NSWindow);
  // Managed: belongs to one Space. Stationary: stays put (not shuffled) in
  // Mission Control. IgnoresCycle: never in Cmd-`. FullScreenNone: cannot
  // become a full-screen Space of its own.
  native.setCollectionBehavior(B::Managed | B::Stationary | B::IgnoresCycle | B::FullScreenNone);
  native.setLevel(25); // NSStatusWindowLevel: above every normal and floating window.
  native.setIgnoresMouseEvents(true);
  native.setHasShadow(false);
  let number = native.windowNumber() as u32;
  // Invisible until it is on its Space, so it never flashes on the wrong one.
  // Never makeKey/activate: that could steal focus or jump Spaces.
  native.setAlphaValue(0.0);
  native.orderFrontRegardless();
  if spaces.move_window_to_space(number, space) {
    native.setAlphaValue(1.0);
    Some(number)
  } else {
    native.orderOut(None);
    None
  }
}
