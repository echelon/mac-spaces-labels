//! Runs JavaScript for Automation through `osascript`.
//!
//! A child process keeps Apple Event failures (app busy, permission prompt
//! pending, app quitting) from ever blocking or crashing the caller, and the
//! Automation permission prompt is still attributed to our app, which is the
//! responsible process.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub fn run_jxa(script: &str, timeout: Duration) -> Result<String, String> {
  let mut child = Command::new("/usr/bin/osascript")
    .args(["-l", "JavaScript", "-e", script])
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .map_err(|e| format!("osascript: {e}"))?;
  // Drain stdout on a thread so a large reply cannot fill the pipe and stall
  // the child while we wait for it to exit.
  let mut stdout = child.stdout.take().expect("piped stdout");
  let reader = std::thread::spawn(move || {
    let mut out = String::new();
    let _ = stdout.read_to_string(&mut out);
    out
  });
  let started = Instant::now();
  let status = loop {
    match child.try_wait() {
      Ok(Some(status)) => break status,
      Ok(None) if started.elapsed() < timeout => std::thread::sleep(Duration::from_millis(10)),
      Ok(None) => {
        let _ = child.kill();
        let _ = child.wait();
        return Err("osascript timed out".into());
      }
      Err(e) => return Err(format!("osascript: {e}")),
    }
  };
  let out = reader.join().unwrap_or_default();
  if status.success() {
    Ok(out)
  } else {
    let mut err = String::new();
    if let Some(mut stderr) = child.stderr.take() {
      let _ = stderr.read_to_string(&mut err);
    }
    Err(err.trim().to_string())
  }
}
