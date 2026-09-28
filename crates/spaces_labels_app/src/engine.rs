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
//! The slower per-window context (tabs, terminals; see `context`) runs on its
//! own thread, poked by the same events, and asks the worker to push results.
//!
//! The worker reads the world without holding the model lock, reconciles, and
//! only then touches windows, because creating a window waits on the main
//! thread and the main thread also takes the lock (menu events, commands).

use crate::model::{self, OverlayState, Sources};
use crate::overlay::{self, Frame};
use crate::settings::Settings;
use app_context::SpaceContext;
use spaces_sys::{AppsBySpace, Snapshot, SpaceId, SpaceKind, Spaces};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI32, Ordering};
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
/// How often the poll loop hit-tests the pointer against clickable overlay
/// regions, in poll ticks (8 x 2 ms: about 60 Hz).
const HOVER_EVERY: u32 = 8;

/// The most recently activated app other than us, to hand focus back to
/// after a click on an overlay activates this app.
static LAST_FRONT_PID: AtomicI32 = AtomicI32::new(0);

pub enum Msg {
  Refresh,
  /// Settings changed or displays moved: recompute window positions.
  Relayout,
  /// New context arrived: only re-send overlay states.
  Push,
}

pub struct Overlay {
  pub label: String,
  pub window_number: u32,
  pub display: String,
  pub sent: Option<OverlayState>,
  /// Where the window is, in global points.
  pub frame: Frame,
  /// The clickable region ("more" link or expanded panel), window-relative,
  /// as reported by the page.
  pub hit: Option<Frame>,
  /// The whole label panel, window-relative: hovering it holds the label.
  pub panel: Option<Frame>,
  /// Whether the pointer was over `panel` at the last hover check.
  pub pointer_inside: bool,
  pub expanded: bool,
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
  /// Per-Space context; `None` until the first context pass finishes.
  pub contexts: Option<HashMap<SpaceId, SpaceContext>>,
  pub titles_readable: bool,
  /// The overlay currently accepting the mouse, if any.
  pub interactive: Option<String>,
  /// Window number -> the vision model's latest description.
  pub vision: HashMap<u32, crate::vision::VisionNote>,
  pub vision_status: crate::vision::VisionStatus,
  /// Desktop names (yours and the model's), by Space UUID.
  pub names: crate::naming::NameStore,
  /// App name -> icon data URL (`None` when it has no icon).
  pub icons: HashMap<String, Option<String>>,
  /// The Spaces on screen (one per display), updated by the poll the moment
  /// a switch is seen, ahead of the worker's full snapshot.
  pub showing: Vec<SpaceId>,
  /// When `settings.json` was last written, to notice outside edits.
  pub settings_modified: Option<std::time::SystemTime>,
}

impl Model {
  pub fn sources(&self) -> Sources<'_> {
    Sources {
      snapshot: &self.snapshot,
      apps: &self.apps,
      contexts: self.contexts.as_ref(),
      titles_readable: self.titles_readable,
      placement: self.settings.placement,
      show_apps: self.settings.show_apps,
      vision: &self.vision,
      vision_status: &self.vision_status,
      names: &self.names,
      aliases: &self.settings.project_names,
      icons: &self.icons,
      showing: &self.showing,
      auto_hide: self.settings.auto_hide,
      timing: self.settings.timing(),
    }
  }

  pub fn title_for(&self, space: &spaces_sys::Space) -> String {
    let context = self.contexts.as_ref().and_then(|all| all.get(&space.id));
    model::titles(space, &self.names, context, &self.settings.project_names).title
  }

  pub fn overlay_by_label(&mut self, label: &str) -> Option<&mut Overlay> {
    self.overlays.values_mut().find(|o| o.label == label)
  }
}

pub struct Engine {
  pub spaces: Spaces,
  pub model: Mutex<Model>,
  pub tx: Mutex<Sender<Msg>>,
  /// Pokes the context thread.
  pub context_tx: Mutex<Sender<()>>,
  pub settings_path: PathBuf,
  /// App config directory: settings, names, context and feedback files.
  pub config_dir: PathBuf,
}

impl Engine {
  pub fn new(config_dir: PathBuf) -> (Self, Receiver<Msg>, Receiver<()>) {
    let settings_path = config_dir.join("settings.json");
    let (tx, rx) = mpsc::channel();
    let (context_tx, context_rx) = mpsc::channel();
    let settings = Settings::load(&settings_path);
    // Write every setting (defaults included) so the file shows what can be
    // tuned when opened from the menu bar.
    settings.save(&settings_path);
    let model = Model {
      settings_modified: Settings::modified(&settings_path),
      settings,
      names: crate::naming::load(&config_dir),
      ..Default::default()
    };
    (
      Self {
        spaces: Spaces::connect(),
        model: Mutex::new(model),
        tx: Mutex::new(tx),
        context_tx: Mutex::new(context_tx),
        settings_path,
        config_dir,
      },
      rx,
      context_rx,
    )
  }

  pub fn send(&self, msg: Msg) {
    if matches!(msg, Msg::Refresh) {
      let _ = self.context_tx.lock().unwrap().send(());
    }
    let _ = self.tx.lock().unwrap().send(msg);
  }

  pub fn state_for_label(&self, label: &str) -> Option<OverlayState> {
    let model = self.model.lock().unwrap();
    let (space, overlay) = model.overlays.iter().find(|(_, o)| o.label == label)?;
    model::overlay_state(&model.sources(), *space, overlay.expanded)
  }

  /// Expands or collapses one overlay, then hands focus back to the app the
  /// click took it from.
  pub fn set_expanded(&self, app: &AppHandle, label: &str, expanded: bool) {
    let target = {
      let mut model = self.model.lock().unwrap();
      let placement = model.settings.placement;
      model.overlay_by_label(label).map(|o| {
        o.expanded = expanded;
        (o.display.clone(), placement)
      })
    };
    if let Some((display, placement)) = target {
      if let Some(frame) = overlay::reposition(app, label, &display, placement, expanded) {
        if let Some(o) = self.model.lock().unwrap().overlay_by_label(label) {
          o.frame = frame;
        }
      }
    }
    restore_focus(app);
    self.send(Msg::Push);
  }

  /// Names a desktop yourself (an empty name returns it to the model's), and
  /// logs the rename with what the model saw and suggested, for evals.
  pub fn rename(&self, app: &AppHandle, label: &str, name: &str, description: &str) {
    let name = name.trim();
    let record = {
      let mut model = self.model.lock().unwrap();
      let Some(space_id) = model
        .overlays
        .iter()
        .find(|(_, o)| o.label == label)
        .map(|(s, _)| *s)
      else {
        return;
      };
      let Some((_, space)) = model.snapshot.space(space_id) else {
        return;
      };
      let (uuid, desktop) = (space.uuid.clone(), model::space_name(space));
      let context = model
        .contexts
        .as_ref()
        .and_then(|all| all.get(&space_id).cloned())
        .map(|c| model::with_vision(c, &model.vision))
        .unwrap_or_default();
      let entry = model.names.entry(uuid.clone()).or_default();
      let ai = entry.ai.clone();
      entry.user = (!name.is_empty()).then(|| crate::naming::UserName {
        name: name.to_string(),
        description: description.trim().to_string(),
        at_unix: crate::naming::now_unix(),
      });
      crate::naming::save(&self.config_dir, &model.names);
      serde_json::json!({
        "at_unix": crate::naming::now_unix(),
        "space_uuid": uuid,
        "desktop": desktop,
        "input": crate::naming::digest(&context),
        "ai": ai,
        "user": (!name.is_empty()).then(|| serde_json::json!({"name": name, "description": description.trim()})),
        "context": context,
      })
    };
    crate::naming::record_feedback(&self.config_dir, &record);
    restore_focus(app);
    self.send(Msg::Refresh);
  }

  pub fn set_hit_rect(&self, label: &str, hit: Option<Frame>, panel: Option<Frame>) {
    if let Some(o) = self.model.lock().unwrap().overlay_by_label(label) {
      o.hit = hit;
      o.panel = panel;
    }
  }

  pub fn update_settings(&self, change: impl FnOnce(&mut Settings)) {
    let mut model = self.model.lock().unwrap();
    change(&mut model.settings);
    model.settings.save(&self.settings_path);
    model.settings_modified = Settings::modified(&self.settings_path);
    drop(model);
    self.send(Msg::Relayout);
  }

  /// Picks up edits to `settings.json` made outside the app (menu bar →
  /// Edit Settings…). Returns whether anything was reloaded.
  pub fn reload_settings_if_changed(&self) -> bool {
    let modified = Settings::modified(&self.settings_path);
    let mut model = self.model.lock().unwrap();
    if modified == model.settings_modified {
      return false;
    }
    model.settings_modified = modified;
    model.settings = Settings::load(&self.settings_path);
    true
  }
}

/// Menu-bar text. macOS hides status items that do not fit (next to the
/// notch, a long title makes the whole item vanish), so keep it short; the
/// overlay shows the full name.
fn tray_title(name: &str) -> String {
  const MAX: usize = 18;
  if name.chars().count() <= MAX {
    return name.to_string();
  }
  let mut short: String = name
    .chars()
    .take(MAX - 1)
    .collect::<String>()
    .trim_end()
    .to_string();
  short.push('…');
  short
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
  let mut tick = 0u32;
  loop {
    tick = tick.wrapping_add(1);
    if tick.is_multiple_of(HOVER_EVERY) {
      update_hover(&app, &engine);
    }
    let active = engine.spaces.active_space();
    if active != last {
      last = active;
      let now = Instant::now();
      // Which Spaces are showing on every display (~100 µs), so the arriving
      // desktop's label starts its show-then-fade and the departing one
      // re-arms for next time.
      let showing: Vec<SpaceId> = engine.spaces.snapshot().current_spaces().collect();
      let (title, changes) = {
        let mut model = engine.model.lock().unwrap();
        model.last_switch = Some((active, now));
        let previous = std::mem::replace(&mut model.showing, showing.clone());
        let mut changes = Vec::new();
        for (space, overlay) in model.overlays.iter_mut() {
          let (was, is) = (previous.contains(space), showing.contains(space));
          if was != is {
            overlay.pointer_inside = false;
            changes.push((overlay.label.clone(), is));
          }
        }
        let title = model
          .snapshot
          .space(active)
          .map(|(_, space)| model.title_for(space));
        (title, changes)
      };
      for (label, is_showing) in changes {
        let _ = app.emit_to(label.as_str(), "space-active", is_showing);
      }
      // Unknown Space (just created): the worker's snapshot will name it.
      if let (Some(title), Some(tray)) = (title, app.tray_by_id(TRAY_ID)) {
        let _ = tray.set_title(Some(tray_title(&title)));
      }
      engine.send(Msg::Refresh);
      if debug_enabled() {
        eprintln!("[poll] active space -> {active}");
      }
    }
    std::thread::sleep(POLL);
  }
}

/// Makes the overlay under the pointer clickable when the pointer is over
/// its clickable region, and every other overlay click-through again.
fn update_hover(app: &AppHandle, engine: &Engine) {
  let (x, y) = spaces_sys::mouse_location();
  let mut model = engine.model.lock().unwrap();
  let showing = model.showing.clone();
  // Tell a showing label when the pointer enters or leaves it (it holds
  // while hovered and fades once the pointer leaves).
  let mut pointer_events = Vec::new();
  for (space, overlay) in model.overlays.iter_mut() {
    let inside = showing.contains(space)
      && overlay
        .panel
        .is_some_and(|panel| panel.offset(overlay.frame).contains(x, y));
    if inside != overlay.pointer_inside {
      overlay.pointer_inside = inside;
      pointer_events.push((overlay.label.clone(), inside));
    }
  }
  if !pointer_events.is_empty() {
    let app = app.clone();
    // Emitting evaluates script in the webview; keep it off the lock.
    std::thread::spawn(move || {
      for (label, inside) in pointer_events {
        let _ = app.emit_to(label.as_str(), "pointer", inside);
      }
    });
  }
  let target = model
    .overlays
    .iter()
    .filter(|(space, _)| showing.contains(space))
    .find_map(|(_, o)| {
      let hit = o.hit?.offset(o.frame);
      hit.contains(x, y).then(|| o.label.clone())
    });
  if target == model.interactive {
    return;
  }
  let previous = std::mem::replace(&mut model.interactive, target.clone());
  drop(model);
  if let Some(label) = previous {
    overlay::set_click_through(app, &label, true);
  }
  if let Some(label) = target {
    overlay::set_click_through(app, &label, false);
  }
}

fn worker_loop(app: AppHandle, rx: Receiver<Msg>) {
  let mut relayout = true;
  loop {
    let mut full = true;
    match rx.recv_timeout(RESCAN) {
      Ok(msg) => {
        relayout |= matches!(msg, Msg::Relayout);
        full = !matches!(msg, Msg::Push);
      }
      Err(RecvTimeoutError::Timeout) => {}
      Err(RecvTimeoutError::Disconnected) => return,
    }
    if app.state::<Engine>().reload_settings_if_changed() {
      relayout = true;
      full = true;
    }
    // Coalesce a burst (a switch fires the poll and a notification) into one pass.
    while let Ok(msg) = rx.try_recv() {
      relayout |= matches!(msg, Msg::Relayout);
      full |= !matches!(msg, Msg::Push);
    }
    if !full {
      push_states(&app, &app.state::<Engine>());
      continue;
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
    .map(|(_, s)| model.title_for(s));

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
  // Icons for apps seen for the first time (rendered below, unlocked).
  let mut new_icons: Vec<(String, i32)> = Vec::new();
  for app_on_space in apps.values().flatten() {
    if !model.icons.contains_key(&app_on_space.name)
      && !new_icons.iter().any(|(n, _)| *n == app_on_space.name)
    {
      new_icons.push((app_on_space.name.clone(), app_on_space.pid));
    }
  }
  model.snapshot = snapshot;
  model.apps = apps;
  let placement = model.settings.placement;
  let relayout_targets: Vec<(String, String, bool)> = if relayout {
    model
      .overlays
      .values()
      .map(|o| (o.label.clone(), o.display.clone(), o.expanded))
      .collect()
  } else {
    Vec::new()
  };
  drop(model);

  if let (Some(title), Some(tray)) = (active_title, app.tray_by_id(TRAY_ID)) {
    let _ = tray.set_title(Some(tray_title(&title)));
  }
  if !new_icons.is_empty() {
    let rendered = crate::icons::render(app, new_icons);
    engine.model.lock().unwrap().icons.extend(rendered);
  }
  for label in to_close {
    overlay::close(app, &label);
  }
  for (label, display, expanded) in relayout_targets {
    if let Some(frame) = overlay::reposition(app, &label, &display, placement, expanded) {
      if let Some(o) = engine.model.lock().unwrap().overlay_by_label(&label) {
        o.frame = frame;
      }
    }
  }
  for Pending { space, display } in to_create {
    let created = overlay::create(app, spaces, &display, placement, space);
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
            frame: created.frame,
            hit: None,
            panel: None,
            pointer_inside: false,
            expanded: false,
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
    let sources = model.sources();
    let mut changed = Vec::new();
    for (space, overlay) in &model.overlays {
      let Some(state) = model::overlay_state(&sources, *space, overlay.expanded) else {
        continue;
      };
      if overlay.sent.as_ref() != Some(&state) {
        changed.push((*space, state));
      }
    }
    for (space, state) in changed {
      let overlay = model.overlays.get_mut(&space).unwrap();
      overlay.sent = Some(state.clone());
      outgoing.push((overlay.label.clone(), state));
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
    let is_space_change = i == 0; // Order of `names` above; 3 is activation.
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
      if i == 3 {
        remember_front_app();
      }
      engine.send(Msg::Refresh);
    });
    let observer =
      unsafe { center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block) };
    // Observers live as long as the app.
    std::mem::forget(observer);
  }
}

/// Records the frontmost app unless it is us. Main thread (notification).
#[cfg(target_os = "macos")]
fn remember_front_app() {
  use objc2_app_kit::NSWorkspace;
  if let Some(front) = NSWorkspace::sharedWorkspace().frontmostApplication() {
    let pid = front.processIdentifier();
    if pid != std::process::id() as i32 {
      LAST_FRONT_PID.store(pid, Ordering::Relaxed);
    }
  }
}

/// A click on an overlay activates this (accessory) app and takes focus from
/// whatever the user was working in. Give it straight back.
#[cfg(target_os = "macos")]
pub fn restore_focus(app: &AppHandle) {
  let pid = LAST_FRONT_PID.load(Ordering::Relaxed);
  if pid == 0 {
    return;
  }
  let _ = app.run_on_main_thread(move || {
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
    if !NSRunningApplication::currentApplication().isActive() {
      return;
    }
    if let Some(previous) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) {
      #[allow(deprecated)]
      previous.activateWithOptions(NSApplicationActivationOptions::empty());
    }
  });
}
