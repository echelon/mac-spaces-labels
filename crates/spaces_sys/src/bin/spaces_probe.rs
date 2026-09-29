//! Diagnostics: `spaces-probe` prints Spaces, apps per Space, and call costs.
//! `spaces-probe watch` logs Space changes seen by a 1 ms poll next to every
//! raw SkyLight notification, to measure which signal arrives first.

use spaces_sys::{ffi, Spaces};
use std::ffi::c_void;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

static START: OnceLock<Instant> = OnceLock::new();

fn ms() -> f64 {
  START.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0
}

fn time<T>(label: &str, runs: u32, mut f: impl FnMut() -> T) -> T {
  let started = Instant::now();
  let mut out = f();
  for _ in 1..runs {
    out = f();
  }
  println!(
    "{label:>22}: {:>9.1} µs/call",
    started.elapsed().as_secs_f64() * 1e6 / runs as f64
  );
  out
}

extern "C" fn on_notify(event: u32, _data: *mut c_void, _len: usize, _user: *mut c_void) {
  let space = Spaces::connect().active_space();
  println!("{:>10.2} ms  notify event {event:<5} active={space}", ms());
}

extern "C" {
  fn CFRunLoopRun();
}

fn main() {
  ms();
  let spaces = Spaces::connect();
  let snapshot = time("snapshot", 200, || spaces.snapshot());
  time("active_space", 10_000, || spaces.active_space());
  let apps = time("apps_by_space", 20, || {
    spaces.apps_by_space(std::process::id() as i32)
  });

  for display in &snapshot.displays {
    println!(
      "\ndisplay {} (current {})",
      display.uuid, display.current_space
    );
    for space in &display.spaces {
      let names: Vec<&str> = apps
        .get(&space.id)
        .into_iter()
        .flatten()
        .map(|a| a.name.as_str())
        .collect();
      let marker = if space.id == display.current_space {
        "*"
      } else {
        " "
      };
      println!(
        " {marker} {:>6} {:?} #{:?}: {}",
        space.id,
        space.kind,
        space.desktop_number,
        names.join(", ")
      );
    }
  }

  if let (Some("owner"), Some(pid)) = (std::env::args().nth(1).as_deref(), std::env::args().nth(2))
  {
    // Which Space each window of a process is on (for checking overlays).
    let pid: i32 = pid.parse().expect("pid");
    for line in spaces.describe_windows(pid) {
      println!("{line}");
    }
    return;
  }
  if std::env::args().nth(1).as_deref() == Some("tags") {
    for id in std::env::args()
      .skip(2)
      .filter_map(|a| a.parse::<u32>().ok())
    {
      println!("{id}: {:064b}", spaces.window_tags(id));
    }
    return;
  }
  if std::env::args().nth(1).as_deref() != Some("watch") {
    return;
  }
  // Register for a wide range of event ids; the interesting ones reveal
  // themselves by firing only around Space switches.
  let skip: Vec<u32> = std::env::args()
    .skip(2)
    .filter_map(|a| a.parse().ok())
    .collect();
  for event in 0..2000u32 {
    if !skip.contains(&event) {
      unsafe { ffi::CGSRegisterNotifyProc(on_notify, event, std::ptr::null_mut()) };
    }
  }
  std::thread::spawn(move || {
    let mut last = spaces.active_space();
    loop {
      let now = spaces.active_space();
      if now != last {
        println!("{:>10.2} ms  POLL active {last} -> {now}", ms());
        last = now;
      }
      std::thread::sleep(Duration::from_millis(1));
    }
  });
  println!("\nwatching; switch Spaces now");
  unsafe { CFRunLoopRun() };
}
