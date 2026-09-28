//! Spaces Labels: a big, transparent, click-through name on every macOS
//! desktop, plus the apps running there and (under "more") their tabs,
//! terminals and window titles. See `engine` for how it stays in sync,
//! `overlay` for why each Space gets its own window, and `context` for the
//! per-window details.

mod context;
mod engine;
mod model;
mod overlay;
mod settings;
mod vision;

use engine::{Engine, TRAY_ID};
use model::{OverlayState, Placement};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{Manager, RunEvent, State, WebviewWindow, Wry};

#[tauri::command]
fn overlay_state(window: WebviewWindow, engine: State<Engine>) -> Option<OverlayState> {
  engine.state_for_label(window.label())
}

#[tauri::command]
fn set_expanded(window: WebviewWindow, engine: State<Engine>, expanded: bool) {
  engine.set_expanded(window.app_handle(), window.label(), expanded);
}

/// The page reports its clickable region (window-relative points), or `None`.
#[tauri::command]
fn set_hit_rect(window: WebviewWindow, engine: State<Engine>, rect: Option<overlay::Frame>) {
  engine.set_hit_rect(window.label(), rect);
}

const SCREEN_RECORDING_SETTINGS: &str =
  "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";

fn build_tray(app: &tauri::App, engine: &Engine, titles_missing: bool) -> tauri::Result<()> {
  let settings = engine.model.lock().unwrap().settings.clone();
  let check = |(placement, name): (Placement, &str)| {
    CheckMenuItem::with_id(
      app,
      placement.id(),
      name,
      true,
      placement == settings.placement,
      None::<&str>,
    )
  };
  let corners: Vec<CheckMenuItem<Wry>> = Placement::CORNERS
    .into_iter()
    .map(check)
    .collect::<Result<_, _>>()?;
  let centered: Vec<CheckMenuItem<Wry>> = Placement::CENTERED
    .into_iter()
    .map(check)
    .collect::<Result<_, _>>()?;
  let separator = PredefinedMenuItem::separator(app)?;
  let mut position_items: Vec<&dyn tauri::menu::IsMenuItem<Wry>> =
    corners.iter().map(|c| c as _).collect();
  position_items.push(&separator);
  position_items.extend(
    centered
      .iter()
      .map(|c| c as &dyn tauri::menu::IsMenuItem<Wry>),
  );
  let position = Submenu::with_items(app, "Label position", true, &position_items)?;
  let placement_items: Vec<CheckMenuItem<Wry>> = corners.into_iter().chain(centered).collect();
  let show_apps = CheckMenuItem::with_id(
    app,
    "show_apps",
    "Show apps on each desktop",
    true,
    settings.show_apps,
    None::<&str>,
  )?;
  let describe = CheckMenuItem::with_id(
    app,
    "vision",
    "Describe windows with local AI",
    true,
    settings.vision,
    None::<&str>,
  )?;
  let quit = MenuItem::with_id(app, "quit", "Quit Spaces Labels", true, Some("Cmd+Q"))?;
  // Window titles need Screen Recording; macOS applies a grant on relaunch.
  let allow_titles = MenuItem::with_id(
    app,
    "allow_titles",
    "Allow window titles (Screen Recording)…",
    true,
    None::<&str>,
  )?;
  let separator = PredefinedMenuItem::separator(app)?;
  let mut items: Vec<&dyn tauri::menu::IsMenuItem<Wry>> = vec![&position, &show_apps, &describe];
  if titles_missing {
    items.push(&allow_titles);
  }
  items.push(&separator);
  items.push(&quit);
  let menu = Menu::with_items(app, &items)?;

  TrayIconBuilder::with_id(TRAY_ID)
    .icon(tauri::image::Image::from_bytes(include_bytes!(
      "../icons/tray.png"
    ))?)
    .icon_as_template(true)
    .title("…")
    .tooltip("Spaces Labels")
    .menu(&menu)
    .show_menu_on_left_click(true)
    .on_menu_event(move |app, event| {
      let engine = app.state::<Engine>();
      match event.id().as_ref() {
        "quit" => app.exit(0),
        "allow_titles" => {
          let _ = std::process::Command::new("/usr/bin/open")
            .arg(SCREEN_RECORDING_SETTINGS)
            .spawn();
        }
        "show_apps" => engine.update_settings(|s| s.show_apps = !s.show_apps),
        "vision" => engine.update_settings(|s| s.vision = !s.vision),
        id => {
          if let Some((placement, _)) = Placement::all().find(|(p, _)| p.id() == id) {
            engine.update_settings(|s| s.placement = placement);
            for item in &placement_items {
              let _ = item.set_checked(item.id().as_ref() == id);
            }
          }
        }
      }
    })
    .build(app)?;
  Ok(())
}

fn main() {
  tauri::Builder::default()
    .invoke_handler(tauri::generate_handler![
      overlay_state,
      set_expanded,
      set_hit_rect
    ])
    .setup(|app| {
      #[cfg(target_os = "macos")]
      app.set_activation_policy(tauri::ActivationPolicy::Accessory);
      let config_dir = app.path().app_config_dir()?;
      let (engine, rx, context_rx) = Engine::new(config_dir.join("settings.json"));
      let titles_missing = !spaces_sys::can_read_titles();
      if titles_missing {
        // Shows the system prompt the first time; afterwards the menu item
        // leads to System Settings.
        spaces_sys::request_title_access();
      }
      build_tray(app, &engine, titles_missing)?;
      app.manage(engine);
      #[cfg(target_os = "macos")]
      engine::observe_workspace(app.handle());
      engine::start(app.handle(), rx);
      context::start(app.handle(), context_rx, config_dir.join("context.json"));
      vision::start(app.handle());
      Ok(())
    })
    .build(tauri::generate_context!())
    .expect("build Spaces Labels")
    .run(|_, event| {
      // Overlays come and go with Spaces; only an explicit Quit exits.
      if let RunEvent::ExitRequested {
        code: None, api, ..
      } = event
      {
        api.prevent_exit();
      }
    });
}
