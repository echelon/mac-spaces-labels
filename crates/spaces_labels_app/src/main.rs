//! Spaces Labels: a big, transparent, click-through name on every macOS
//! desktop, plus the apps running there. See `engine` for how it stays in sync
//! and `overlay` for why each Space gets its own window.

mod engine;
mod model;
mod overlay;
mod settings;

use engine::{Engine, TRAY_ID};
use model::{Corner, OverlayState};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{Manager, RunEvent, State, WebviewWindow, Wry};

#[tauri::command]
fn overlay_state(window: WebviewWindow, engine: State<Engine>) -> Option<OverlayState> {
  engine.state_for_label(window.label())
}

fn build_tray(app: &tauri::App, engine: &Engine) -> tauri::Result<()> {
  let settings = engine.model.lock().unwrap().settings.clone();
  let corners: Vec<CheckMenuItem<Wry>> = Corner::ALL
    .iter()
    .map(|(corner, name)| {
      CheckMenuItem::with_id(
        app,
        corner.id(),
        *name,
        true,
        *corner == settings.corner,
        None::<&str>,
      )
    })
    .collect::<Result<_, _>>()?;
  let corner_refs: Vec<&dyn tauri::menu::IsMenuItem<Wry>> =
    corners.iter().map(|c| c as _).collect();
  let position = Submenu::with_items(app, "Label position", true, &corner_refs)?;
  let show_apps = CheckMenuItem::with_id(
    app,
    "show_apps",
    "Show apps on each desktop",
    true,
    settings.show_apps,
    None::<&str>,
  )?;
  let quit = MenuItem::with_id(app, "quit", "Quit Spaces Labels", true, Some("Cmd+Q"))?;
  let menu = Menu::with_items(
    app,
    &[
      &position,
      &show_apps,
      &PredefinedMenuItem::separator(app)?,
      &quit,
    ],
  )?;

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
        "show_apps" => engine.update_settings(|s| s.show_apps = !s.show_apps),
        id => {
          if let Some((corner, _)) = Corner::ALL.iter().find(|(c, _)| c.id() == id) {
            engine.update_settings(|s| s.corner = *corner);
            for item in &corners {
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
    .invoke_handler(tauri::generate_handler![overlay_state])
    .setup(|app| {
      #[cfg(target_os = "macos")]
      app.set_activation_policy(tauri::ActivationPolicy::Accessory);
      let settings_path = app.path().app_config_dir()?.join("settings.json");
      let (engine, rx) = Engine::new(settings_path);
      build_tray(app, &engine)?;
      app.manage(engine);
      #[cfg(target_os = "macos")]
      engine::observe_workspace(app.handle());
      engine::start(app.handle(), rx);
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
