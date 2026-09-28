//! Local vision-model descriptions of windows ("what is this window doing").
//!
//! A llama.cpp server (Homebrew `llama-server`, Metal) runs as a child
//! process bound to 127.0.0.1 with a small VLM (Qwen3-VL-2B). Nothing leaves
//! the machine. It is kept polite:
//! * one screenshot at a time, with a [`COOLDOWN`] between inferences, so the
//!   GPU is busy a small fraction of the time;
//! * windows on the showing Spaces first; each window is re-described at most
//!   every [`REFRESH_SHOWING`]/[`REFRESH_HIDDEN`] unless its title changes;
//! * paused under serious thermal pressure or Low Power Mode, and when Screen
//!   Recording (needed for screenshots) is not granted;
//! * the server runs at low priority, is stopped after [`IDLE_SHUTDOWN`] to
//!   free its ~3 GB, and a watchdog kills it if this app dies.
//!
//! Screenshots come from `screencapture -l <window>` (works for windows on
//! other Spaces) downscaled by `sips`, then are deleted.

use crate::engine::{debug_enabled, Engine, Msg};
use app_context::WindowContext;
use serde::Serialize;
use spaces_sys::SpaceId;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

pub const MODEL: &str = "Qwen3VL-2B-Instruct-Q8_0.gguf";
pub const PROJECTOR: &str = "mmproj-Qwen3VL-2B-Instruct-Q8_0.gguf";
/// Caps the vision tokens per screenshot: the main speed/quality lever.
const MAX_IMAGE_TOKENS: u32 = 1024;
/// Longest side of the screenshot sent to the model, in pixels.
const IMAGE_SIDE: u32 = 1024;
const COOLDOWN: Duration = Duration::from_secs(3);
const REFRESH_SHOWING: Duration = Duration::from_secs(60);
const REFRESH_HIDDEN: Duration = Duration::from_secs(300);
const IDLE_SHUTDOWN: Duration = Duration::from_secs(600);
/// Windows smaller than this (points) are palettes and popups.
const MIN_SIDE: f64 = 200.0;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct VisionNote {
  pub text: String,
  /// The window title when described; a new title means a new description.
  #[serde(skip)]
  pub title: Option<String>,
  #[serde(skip)]
  pub at: Option<Instant>,
  pub seconds: f32,
}

/// What the overlays show about the vision model.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct VisionStatus {
  pub enabled: bool,
  /// Why it is not describing anything, if it is not.
  pub blocked: Option<String>,
}

pub fn models_dir(app: &AppHandle) -> Option<PathBuf> {
  app.path().app_config_dir().ok().map(|d| d.join("models"))
}

fn llama_server() -> Option<&'static str> {
  [
    "/opt/homebrew/bin/llama-server",
    "/usr/local/bin/llama-server",
  ]
  .into_iter()
  .find(|p| Path::new(p).exists())
}

pub fn start(app: &AppHandle) {
  let app = app.clone();
  std::thread::Builder::new()
    .name("space-vision".into())
    .spawn(move || run(app))
    .expect("spawn vision");
}

struct Server {
  child: Child,
  port: u16,
  last_used: Instant,
}

impl Drop for Server {
  fn drop(&mut self) {
    // Killing the watchdog shell runs its trap, which kills llama-server.
    let _ = self.child.kill();
    let _ = self.child.wait();
  }
}

fn run(app: AppHandle) {
  let engine = app.state::<Engine>();
  let Some(models) = models_dir(&app) else {
    return;
  };
  let scratch = std::env::temp_dir().join(format!("spaces-labels-{}", std::process::id()));
  let _ = std::fs::create_dir_all(&scratch);
  let mut server: Option<Server> = None;
  loop {
    std::thread::sleep(Duration::from_secs(1));
    let blocked = blocked_reason(&engine, &models);
    set_status(&engine, blocked.clone());
    if blocked.is_some() {
      server = None;
      continue;
    }
    let Some(target) = next_window(&engine) else {
      if server
        .as_ref()
        .is_some_and(|s| s.last_used.elapsed() > IDLE_SHUTDOWN)
      {
        server = None;
      }
      continue;
    };
    if server.is_none() {
      match start_server(&models) {
        Ok(started) => server = Some(started),
        Err(e) => {
          set_status(&engine, Some(format!("model server failed: {e}")));
          std::thread::sleep(Duration::from_secs(30));
          continue;
        }
      }
    }
    let port = server.as_ref().unwrap().port;
    let started = Instant::now();
    let result = capture(target.id, &scratch).and_then(|jpeg| describe(port, &target, &jpeg));
    let seconds = started.elapsed().as_secs_f32();
    if let Some(s) = server.as_mut() {
      s.last_used = Instant::now();
    }
    match result {
      Ok(text) => {
        if debug_enabled() {
          eprintln!(
            "[vision] {} {:?} ({seconds:.2}s): {text}",
            target.app, target.title
          );
        }
        engine.model.lock().unwrap().vision.insert(
          target.id,
          VisionNote {
            text,
            title: target.title.clone(),
            at: Some(Instant::now()),
            seconds,
          },
        );
        engine.send(Msg::Push);
      }
      Err(e) => {
        if debug_enabled() {
          eprintln!("[vision] {} failed: {e}", target.app);
        }
        // Remember the failure too, so one bad window cannot hog the loop.
        engine.model.lock().unwrap().vision.insert(
          target.id,
          VisionNote {
            text: format!("(no description: {e})"),
            title: target.title.clone(),
            at: Some(Instant::now()),
            seconds,
          },
        );
      }
    }
    std::thread::sleep(COOLDOWN);
  }
}

fn set_status(engine: &Engine, blocked: Option<String>) {
  let mut model = engine.model.lock().unwrap();
  let status = VisionStatus {
    enabled: model.settings.vision,
    blocked,
  };
  if model.vision_status != status {
    model.vision_status = status;
    drop(model);
    engine.send(Msg::Push);
  }
}

fn blocked_reason(engine: &Engine, models: &Path) -> Option<String> {
  if !engine.model.lock().unwrap().settings.vision {
    return Some("off (menu bar → Describe windows)".into());
  }
  if llama_server().is_none() {
    return Some("llama.cpp is not installed (brew install llama.cpp)".into());
  }
  if !models.join(MODEL).exists() || !models.join(PROJECTOR).exists() {
    return Some(format!("model not installed in {}", models.display()));
  }
  if !spaces_sys::can_read_titles() {
    return Some("needs Screen Recording".into());
  }
  system_pressure()
}

/// Backs off when the Mac is hot or saving power.
#[cfg(target_os = "macos")]
fn system_pressure() -> Option<String> {
  use objc2_foundation::{NSProcessInfo, NSProcessInfoThermalState};
  let info = NSProcessInfo::processInfo();
  let thermal = info.thermalState();
  if thermal == NSProcessInfoThermalState::Serious || thermal == NSProcessInfoThermalState::Critical
  {
    return Some("paused: the Mac is running hot".into());
  }
  if info.isLowPowerModeEnabled() {
    return Some("paused: Low Power Mode".into());
  }
  None
}

struct Target {
  id: u32,
  app: String,
  title: Option<String>,
}

/// The most useful window to describe next, or `None` if all are fresh.
fn next_window(engine: &Engine) -> Option<Target> {
  let model = engine.model.lock().unwrap();
  let contexts = model.contexts.as_ref()?;
  let showing: Vec<SpaceId> = model.snapshot.current_spaces().collect();
  let mut order: Vec<(bool, &WindowContext, &str)> = Vec::new();
  for (space, context) in contexts {
    for app in &context.apps {
      for window in &app.windows {
        order.push((showing.contains(space), window, &app.name));
      }
    }
  }
  // Showing Spaces first; within each group keep front-to-back order.
  order.sort_by_key(|(on_screen, _, _)| !*on_screen);
  order.into_iter().find_map(|(on_screen, window, app)| {
    if window.width < MIN_SIDE || window.height < MIN_SIDE {
      return None;
    }
    let stale_after = if on_screen {
      REFRESH_SHOWING
    } else {
      REFRESH_HIDDEN
    };
    let fresh = model.vision.get(&window.id).is_some_and(|note| {
      note.title == window.title && note.at.is_some_and(|at| at.elapsed() < stale_after)
    });
    (!fresh).then(|| Target {
      id: window.id,
      app: app.to_string(),
      title: window.title.clone(),
    })
  })
}

fn start_server(models: &Path) -> Result<Server, String> {
  let binary = llama_server().ok_or("llama-server not found")?;
  let port = TcpListener::bind("127.0.0.1:0")
    .and_then(|l| l.local_addr())
    .map_err(|e| e.to_string())?
    .port();
  let quote = |p: &Path| format!("'{}'", p.display().to_string().replace('\'', r"'\''"));
  // The shell is a watchdog: it stops llama-server when this app exits (even
  // if it crashes) or when the shell itself is killed.
  let script = format!(
    "{binary} -m {model} --mmproj {projector} --host 127.0.0.1 --port {port} \
     --ctx-size 4096 --parallel 1 --threads 4 --prio -1 --no-webui \
     --image-max-tokens {MAX_IMAGE_TOKENS} --reasoning off >/dev/null 2>&1 & S=$!; \
     trap 'kill $S 2>/dev/null' EXIT TERM; \
     while kill -0 {parent} 2>/dev/null && kill -0 $S 2>/dev/null; do sleep 2; done",
    model = quote(&models.join(MODEL)),
    projector = quote(&models.join(PROJECTOR)),
    parent = std::process::id(),
  );
  let child = Command::new("/bin/sh")
    .args(["-c", &script])
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .map_err(|e| e.to_string())?;
  let server = Server {
    child,
    port,
    last_used: Instant::now(),
  };
  let deadline = Instant::now() + Duration::from_secs(90);
  while Instant::now() < deadline {
    if matches!(
      http(port, "GET", "/health", None, Duration::from_secs(2)),
      Ok((200, _))
    ) {
      return Ok(server);
    }
    std::thread::sleep(Duration::from_millis(300));
  }
  Err("did not become ready".into())
}

/// A downscaled JPEG of one window, wherever it is (other Spaces included).
fn capture(window: u32, scratch: &Path) -> Result<Vec<u8>, String> {
  let path = scratch.join(format!("w{window}.jpg"));
  let captured = Command::new("/usr/sbin/screencapture")
    .args(["-x", "-o", "-t", "jpg", "-l", &window.to_string()])
    .arg(&path)
    .status()
    .map_err(|e| e.to_string())?;
  if !captured.success() || !path.exists() {
    return Err("screenshot failed".into());
  }
  let _ = Command::new("/usr/bin/sips")
    .args(["-Z", &IMAGE_SIDE.to_string(), "-s", "formatOptions", "80"])
    .arg(&path)
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .status();
  let bytes = std::fs::read(&path).map_err(|e| e.to_string());
  let _ = std::fs::remove_file(&path);
  bytes
}

fn describe(port: u16, target: &Target, jpeg: &[u8]) -> Result<String, String> {
  let prompt =
    format!(
    "Screenshot of the macOS app \"{}\"{}. In one sentence of at most 25 words, say what the user \
     is doing here: the project, file, web page or task. No preamble.",
    target.app,
    target.title.as_deref().map(|t| format!(", window title \"{t}\"")).unwrap_or_default(),
  );
  let body = serde_json::json!({
    "messages": [{"role": "user", "content": [
      {"type": "image_url", "image_url": {"url": format!("data:image/jpeg;base64,{}", base64(jpeg))}},
      {"type": "text", "text": prompt},
    ]}],
    "max_tokens": 60,
    "temperature": 0,
  })
  .to_string();
  let (status, reply) = http(
    port,
    "POST",
    "/v1/chat/completions",
    Some(&body),
    Duration::from_secs(60),
  )?;
  if status != 200 {
    return Err(format!("model server returned {status}"));
  }
  let reply: serde_json::Value = serde_json::from_slice(&reply).map_err(|e| e.to_string())?;
  reply["choices"][0]["message"]["content"]
    .as_str()
    .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
    .filter(|t| !t.is_empty())
    .ok_or_else(|| "empty reply".into())
}

/// Minimal HTTP/1.1 to the loopback model server (no client library needed).
fn http(
  port: u16,
  method: &str,
  path: &str,
  body: Option<&str>,
  timeout: Duration,
) -> Result<(u16, Vec<u8>), String> {
  let mut stream = TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
  stream
    .set_read_timeout(Some(timeout))
    .map_err(|e| e.to_string())?;
  let body = body.unwrap_or("");
  let request = format!(
    "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
    body.len()
  );
  stream
    .write_all(request.as_bytes())
    .map_err(|e| e.to_string())?;
  let mut response = Vec::new();
  stream
    .read_to_end(&mut response)
    .map_err(|e| e.to_string())?;
  let split = response
    .windows(4)
    .position(|w| w == b"\r\n\r\n")
    .ok_or("bad response")?;
  let head = String::from_utf8_lossy(&response[..split]);
  let status = head
    .split_whitespace()
    .nth(1)
    .and_then(|s| s.parse().ok())
    .ok_or("bad status")?;
  let mut payload = response[split + 4..].to_vec();
  if head
    .to_ascii_lowercase()
    .contains("transfer-encoding: chunked")
  {
    payload = dechunk(&payload);
  }
  Ok((status, payload))
}

fn dechunk(mut data: &[u8]) -> Vec<u8> {
  let mut out = Vec::new();
  while let Some(line_end) = data.windows(2).position(|w| w == b"\r\n") {
    let size =
      usize::from_str_radix(String::from_utf8_lossy(&data[..line_end]).trim(), 16).unwrap_or(0);
    if size == 0 {
      break;
    }
    let start = line_end + 2;
    out.extend_from_slice(&data[start..(start + size).min(data.len())]);
    data = &data[(start + size + 2).min(data.len())..];
  }
  out
}

fn base64(bytes: &[u8]) -> String {
  const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
  for chunk in bytes.chunks(3) {
    let n = chunk
      .iter()
      .enumerate()
      .fold(0u32, |n, (i, b)| n | (*b as u32) << (16 - 8 * i));
    for i in 0..4 {
      if i <= chunk.len() {
        out.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
      } else {
        out.push('=');
      }
    }
  }
  out
}

#[cfg(test)]
mod tests {
  #[test]
  fn base64_matches_rfc4648() {
    assert_eq!(super::base64(b""), "");
    assert_eq!(super::base64(b"f"), "Zg==");
    assert_eq!(super::base64(b"fo"), "Zm8=");
    assert_eq!(super::base64(b"foo"), "Zm9v");
    assert_eq!(super::base64(b"foobar"), "Zm9vYmFy");
  }
}
