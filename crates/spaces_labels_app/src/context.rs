//! The context thread: what each window is doing (tabs, terminals, tmux,
//! titles), collected off the UI path because it takes AppleScript round
//! trips of ~100 ms. Runs every [`PERIOD`], and on Space switches and app
//! launches/activations, at most once per [`MIN_GAP`]. Results go to the
//! overlays and to `context.json`, which keeps the latest picture of every
//! desktop for later use (e.g. naming desktops with a local model).

use crate::engine::{debug_enabled, Engine, Msg};
use crate::model::space_name;
use app_context::Collector;
use serde_json::json;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

const PERIOD: Duration = Duration::from_secs(5);
const MIN_GAP: Duration = Duration::from_millis(1000);

pub fn start(app: &AppHandle, rx: Receiver<()>, out: PathBuf) {
  let app = app.clone();
  std::thread::Builder::new()
    .name("space-context".into())
    .spawn(move || run(app, rx, out))
    .expect("spawn context");
}

fn run(app: AppHandle, rx: Receiver<()>, out: PathBuf) {
  let engine = app.state::<Engine>();
  let mut collector = Collector::default();
  let mut last_run: Option<Instant> = None;
  loop {
    if let Err(RecvTimeoutError::Disconnected) = rx.recv_timeout(PERIOD) {
      return;
    }
    if let Some(gap) = last_run.and_then(|at| MIN_GAP.checked_sub(at.elapsed())) {
      std::thread::sleep(gap);
    }
    while rx.try_recv().is_ok() {}
    last_run = Some(Instant::now());

    let windows = engine.spaces.windows(std::process::id() as i32);
    let contexts = collector.collect(&windows);
    if debug_enabled() {
      eprintln!(
        "[context] {:.0} ms, titles readable: {}, errors: {:?}",
        last_run.unwrap().elapsed().as_secs_f64() * 1e3,
        contexts.titles_readable,
        contexts.errors
      );
    }
    let changed = {
      let mut model = engine.model.lock().unwrap();
      let changed = model.contexts.as_ref() != Some(&contexts.by_space)
        || model.titles_readable != contexts.titles_readable;
      model.contexts = Some(contexts.by_space);
      model.titles_readable = contexts.titles_readable;
      changed
    };
    if changed {
      engine.send(Msg::Push);
      save(&engine, &out);
    }
  }
}

/// Writes every desktop's latest context, in Mission Control order.
fn save(engine: &Engine, out: &PathBuf) {
  let json = {
    let model = engine.model.lock().unwrap();
    let Some(contexts) = model.contexts.as_ref() else {
      return;
    };
    let spaces: Vec<_> = model
      .snapshot
      .displays
      .iter()
      .flat_map(|d| d.spaces.iter().map(move |s| (d, s)))
      .map(|(display, space)| {
        json!({
          "space_id": space.id,
          "space_uuid": space.uuid,
          "display": display.uuid,
          "name": space_name(space),
          "apps": contexts.get(&space.id).map(|c| &c.apps),
        })
      })
      .collect();
    let updated = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map(|d| d.as_secs())
      .unwrap_or(0);
    json!({ "updated_unix": updated, "spaces": spaces })
  };
  if let Ok(bytes) = serde_json::to_vec_pretty(&json) {
    let _ = std::fs::write(out, bytes);
  }
}
