//! Real macOS app icons for the overlay's app chips, rendered once per app
//! to a small PNG data URL and cached by app name.

use std::collections::HashMap;
use std::sync::mpsc;
use std::time::Duration;
use tauri::AppHandle;

const SIDE: f64 = 64.0;

/// Icons for the given (app name, pid) pairs that are not cached yet.
/// Rendering needs AppKit's main thread, so this waits for it; call it from
/// a worker thread, never the main thread.
pub fn render(app: &AppHandle, wanted: Vec<(String, i32)>) -> HashMap<String, Option<String>> {
  if wanted.is_empty() {
    return HashMap::new();
  }
  let (tx, rx) = mpsc::sync_channel(1);
  let _ = app.run_on_main_thread(move || {
    let icons = wanted
      .into_iter()
      .map(|(name, pid)| (name, unsafe { icon_data_url(pid) }))
      .collect();
    let _ = tx.send(icons);
  });
  rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default()
}

/// Main thread only.
unsafe fn icon_data_url(pid: i32) -> Option<String> {
  use objc2::AnyThread;
  use objc2_app_kit::{
    NSBitmapImageFileType, NSBitmapImageRep, NSCompositingOperation, NSImage, NSRunningApplication,
  };
  use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize};

  let icon = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)?.icon()?;
  let size = NSSize::new(SIDE, SIDE);
  let small = NSImage::initWithSize(NSImage::alloc(), size);
  #[allow(deprecated)]
  small.lockFocus();
  icon.drawInRect_fromRect_operation_fraction(
    NSRect::new(NSPoint::new(0.0, 0.0), size),
    NSRect::ZERO,
    NSCompositingOperation::SourceOver,
    1.0,
  );
  #[allow(deprecated)]
  small.unlockFocus();
  let tiff = small.TIFFRepresentation()?;
  let bitmap = NSBitmapImageRep::imageRepWithData(&tiff)?;
  let png =
    bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())?;
  Some(format!(
    "data:image/png;base64,{}",
    crate::vision::base64(&png.to_vec())
  ))
}
