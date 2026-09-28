//! Keeps overlay windows in step with the Spaces and the apps on them.
//!
//! Three inputs feed one worker thread through a channel:
//! * a poll of the active Space every [`POLL`] (the call is a shared-memory
//!   read costing well under a microsecond), which also updates the tray title
//!   directly so the menu bar reacts within milliseconds;
//! * `NSWorkspace` notifications (Space changed, apps launched, quit,
//!   activated, hidden), for prompt app-list updates;
//! * a [`RESCAN`] timer, for changes nothing announces (a window opened,
//!   closed, or dragged to another Space; Spaces added or reordered).
//!
//! The worker reads the world without holding the model lock, reconciles, and
//! only then touches windows, because creating a window waits on the main
//! thread and the main thread also takes the lock (menu events, commands).

use crate::model::{self, OverlayState};
use crate::overlay;
use crate::settings::Settings;
use spaces_sys::{AppsBySpace, Snapshot, SpaceId, SpaceKind, Spaces};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

pub const POLL: Duration = Duration::from_millis(2);
const RESCAN: Duration = Duration::from_millis(750);
/// After a failed placement, wait this long before trying that Space again so
/// a persistent failure can never turn into a window-creation loop.
const RETRY_AFTER: Duration = Duration::from_secs(30);
pub const TRAY_ID: &str = "main";

pub enum Msg {
  Refresh,
  /// Settings changed or displays moved: recompute window positions.
  Relayout,
}

pub struct Overlay {
  pub label: String,
  pub window_number: u32,
  pub display: String,
  pub sent: Option<OverlayState>,
}

#[derive(Default)]
pub struct Model {
  pub snapshot: Snapshot,
  pub apps: AppsBySpace,
  pub overlays: HashMap<SpaceId, Overlay>,
  pub settings: Settings,
  /// Spaces whose overlay could not be placed, and when that last happened.
  pub failed: HashMap<SpaceId, Instant>,
  /// When the poll last saw the Space change, for latency diagnostics.
  pub last_switch: Option<(SpaceId, Instant)>,
}

pub struct Engine {
  pub spaces: Spaces,
  pub model: Mutex<Model>,
  pub tx: Mutex<Sender<Msg>>,
  pub settings_path: PathBuf,
}

impl Engine {
  pub fn new(settings_path: PathBuf) -> (Self, Receiver<Msg>) {
    let (tx, rx) = mpsc::channel();
    let settings = Settings::load(&settings_path);
    let model = Model {
      settings,
      ..Default::default()
    };
    (
      Self {
        spaces: Spaces::connect(),
        model: Mutex::new(model),
        tx: Mutex::new(tx),
        settings_path,
      },
      rx,
    )
  }

  pub fn send(&self, msg: Msg) {
    let _ = self.tx.lock().unwrap().send(msg);
  }

  pub fn state_for_label(&self, label: &str) -> Option<OverlayState> {
    let model = self.model.lock().unwrap();
    let (space, _) = model.overlays.iter().find(|(_, o)| o.label == label)?;
    let s = &model.settings;
    model::overlay_state(&model.snapshot, &model.apps, *space, s.corner, s.show_apps)
  }

  pub fn update_settings(&self, change: impl FnOnce(&mut Settings)) {
    let mut model = self.model.lock().unwrap();
    change(&mut model.settings);
    model.settings.save(&self.settings_path);
    drop(model);
    self.send(Msg::Relayout);
  }
}

pub fn debug_enabled() -> bool {
  std::env::var_os("SPACES_LABELS_DEBUG").is_some()
}

pub fn start(app: &AppHandle, rx: Receiver<Msg>) {
  let handle = app.clone();
  std::thread::Builder::new()
    .name("space-poll".into())
    .spawn(move || poll_loop(handle))
    .expect("spawn poll");
  let handle = app.clone();
  std::thread::Builder::new()
    .name("space-worker".into())
    .spawn(move || worker_loop(handle, rx))
    .expect("spawn worker");
}

fn poll_loop(app: AppHandle) {
  let engine = app.state::<Engine>();
  let mut last = 0;
  loop {
    let active = engine.spaces.active_space();
    if active != last {
      last = active;
      let now = Instant::now();
      let title = {
        let mut model = engine.model.lock().unwrap();
        model.last_switch = Some((active, now));
        model
          .snapshot
          .space(active)
          .map(|(_, space)| model::space_name(space))
      };
      // Unknown Space (just created): the worker's snapshot will name it.
      if let (Some(title), Some(tray)) = (title, app.tray_by_id(TRAY_ID)) {
        let _ = tray.set_title(Some(title));
      }
      engine.send(Msg::Refresh);
      if debug_enabled() {
        eprintln!("[poll] active space -> {active}");
      }
    }
    std::thread::sleep(POLL);
  }
}

fn worker_loop(app: AppHandle, rx: Receiver<Msg>) {
  let mut relayout = true;
  loop {
    match rx.recv_timeout(RESCAN) {
      Ok(msg) => relayout |= matches!(msg, Msg::Relayout),
      Err(RecvTimeoutError::Timeout) => {}
      Err(RecvTimeoutError::Disconnected) => return,
    }
    // Coalesce a burst (a switch fires the poll and a notification) into one pass.
    while let Ok(msg) = rx.try_recv() {
      relayout |= matches!(msg, Msg::Relayout);
    }
    let started = Instant::now();
    reconcile(&app, std::mem::take(&mut relayout));
    if debug_enabled() {
      eprintln!(
        "[worker] reconcile {:.2} ms",
        started.elapsed().as_secs_f64() * 1000.0
      );
    }
  }
}

struct Pending {
  space: SpaceId,
  display: String,
}

fn reconcile(app: &AppHandle, relayout: bool) {
  let engine = app.state::<Engine>();
  let spaces = engine.spaces;
  let snapshot = spaces.snapshot();
  let apps = spaces.apps_by_space(std::process::id() as i32);

  let mut model = engine.model.lock().unwrap();
  let active_title = snapshot
    .space(spaces.active_space())
    .map(|(_, s)| model::space_name(s));

  // Drop overlays whose Space is gone, or whose window is not (only) on its
  // Space: a switch raced its creation, or the window was closed.
  let mut to_close = Vec::new();
  model.overlays.retain(|space, overlay| {
    let valid = snapshot
      .space(*space)
      .is_some_and(|(d, s)| s.kind == SpaceKind::Desktop && d.uuid == overlay.display)
      && spaces.spaces_for_window(overlay.window_number) == [*space];
    if !valid {
      to_close.push(overlay.label.clone());
    }
    valid
  });

  let mut to_create = Vec::new();
  for display in &snapshot.displays {
    for space in display
      .spaces
      .iter()
      .filter(|s| s.kind == SpaceKind::Desktop)
    {
      let backing_off = model
        .failed
        .get(&space.id)
        .is_some_and(|at| at.elapsed() < RETRY_AFTER);
      if !model.overlays.contains_key(&space.id) && !backing_off {
        to_create.push(Pending {
          space: space.id,
          display: display.uuid.clone(),
        });
      }
    }
  }
  model.snapshot = snapshot;
  model.apps = apps;
  let corner = model.settings.corner;
  let relayout_targets: Vec<(String, String)> = if relayout {
    model
      .overlays
      .values()
      .map(|o| (o.label.clone(), o.display.clone()))
      .collect()
  } else {
    Vec::new()
  };
  drop(model);

  if let (Some(title), Some(tray)) = (active_title, app.tray_by_id(TRAY_ID)) {
    let _ = tray.set_title(Some(title));
  }
  for label in to_close {
    overlay::close(app, &label);
  }
  for (label, display) in relayout_targets {
    overlay::reposition(app, &label, &display, corner);
  }
  for Pending { space, display } in to_create {
    let created = overlay::create(app, spaces, &display, corner, space);
    let mut model = engine.model.lock().unwrap();
    match created {
      Some(created) => {
        model.failed.remove(&space);
        model.overlays.insert(
          space,
          Overlay {
            label: created.label,
            window_number: created.window_number,
            display,
            sent: None,
          },
        );
      }
      None => {
        model.failed.insert(space, Instant::now());
        if debug_enabled() {
          eprintln!(
            "[worker] could not place an overlay on space {space}; retrying in {RETRY_AFTER:?}"
          );
        }
      }
    }
  }

  push_states(app, &engine);
}

/// Sends each overlay its state, only when it changed.
fn push_states(app: &AppHandle, engine: &Engine) {
  let mut outgoing = Vec::new();
  {
    let mut model = engine.model.lock().unwrap();
    let model = &mut *model;
    let (corner, show_apps) = (model.settings.corner, model.settings.show_apps);
    for (space, overlay) in model.overlays.iter_mut() {
      let Some(state) =
        model::overlay_state(&model.snapshot, &model.apps, *space, corner, show_apps)
      else {
        continue;
      };
      if overlay.sent.as_ref() != Some(&state) {
        overlay.sent = Some(state.clone());
        outgoing.push((overlay.label.clone(), state));
      }
    }
  }
  for (label, state) in outgoing {
    let _ = app.emit_to(label.as_str(), "overlay-state", state);
  }
}

/// Registers `NSWorkspace` observers. Main thread, once.
#[cfg(target_os = "macos")]
pub fn observe_workspace(app: &AppHandle) {
  use block2::RcBlock;
  use objc2_app_kit::{
    NSWorkspace, NSWorkspaceActiveSpaceDidChangeNotification,
    NSWorkspaceDidActivateApplicationNotification, NSWorkspaceDidHideApplicationNotification,
    NSWorkspaceDidLaunchApplicationNotification, NSWorkspaceDidTerminateApplicationNotification,
    NSWorkspaceDidUnhideApplicationNotification,
  };
  use objc2_foundation::NSNotification;
  use std::ptr::NonNull;

  let center = NSWorkspace::sharedWorkspace().notificationCenter();
  let names = unsafe {
    [
      NSWorkspaceActiveSpaceDidChangeNotification,
      NSWorkspaceDidLaunchApplicationNotification,
      NSWorkspaceDidTerminateApplicationNotification,
      NSWorkspaceDidActivateApplicationNotification,
      NSWorkspaceDidHideApplicationNotification,
      NSWorkspaceDidUnhideApplicationNotification,
    ]
  };
  for (i, name) in names.into_iter().enumerate() {
    let app = app.clone();
    let is_space_change = i == 0;
    let block = RcBlock::new(move |_: NonNull<NSNotification>| {
      let engine = app.state::<Engine>();
      if is_space_change && debug_enabled() {
        let active = engine.spaces.active_space();
        match engine.model.lock().unwrap().last_switch {
          Some((space, at)) if space == active => eprintln!(
            "[notify] NSWorkspace space change arrived {:.1} ms after the poll saw it",
            at.elapsed().as_secs_f64() * 1000.0
          ),
          _ => eprintln!("[notify] NSWorkspace space change arrived before the poll saw it"),
        }
      }
      engine.send(Msg::Refresh);
    });
    let observer =
      unsafe { center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block) };
    // Observers live as long as the app.
    std::mem::forget(observer);
  }
}
