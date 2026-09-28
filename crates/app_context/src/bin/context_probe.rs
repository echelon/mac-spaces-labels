//! Prints each Space's context as JSON, with timings. `--no-chrome` skips
//! Chrome, whose first query shows an Automation prompt for whatever process
//! runs this (e.g. your terminal).

use spaces_sys::Spaces;
use std::time::Instant;

fn main() {
  let skip_chrome = std::env::args().any(|a| a == "--no-chrome");
  if std::env::args().any(|a| a == "--firefox") {
    // Bounds from both sides, to debug window matching.
    for w in Spaces::connect().windows(0) {
      if app_context::FIREFOX_APPS.contains(&w.app.as_str()) {
        println!("window  {:?} {:?}", w.bounds, w.title);
      }
    }
    for w in app_context::browser::FirefoxSessions::default().windows() {
      println!(
        "session {:?} {:?} ({} tabs)",
        w.bounds,
        w.title,
        w.tabs.len()
      );
    }
    return;
  }
  let spaces = Spaces::connect();
  let started = Instant::now();
  let mut windows = spaces.windows(std::process::id() as i32);
  if skip_chrome {
    windows.retain(|w| w.app != app_context::CHROME);
  }
  let listed = started.elapsed();
  let mut collector = app_context::Collector::default();
  let contexts = collector.collect(&windows);
  let collected = started.elapsed() - listed;
  let snapshot = spaces.snapshot();
  for display in &snapshot.displays {
    for space in &display.spaces {
      if let Some(context) = contexts.by_space.get(&space.id) {
        println!("== Desktop {:?} ({})", space.desktop_number, space.id);
        println!("{}", serde_json::to_string_pretty(context).unwrap());
      }
    }
  }
  eprintln!(
    "windows {:.1} ms, context {:.1} ms, titles readable: {}, errors: {:?}",
    listed.as_secs_f64() * 1e3,
    collected.as_secs_f64() * 1e3,
    contexts.titles_readable,
    contexts.errors
  );
}
