# Spaces Labels

A transparent, click-through label on every macOS desktop (Space) naming it and
listing the apps running there, plus the current desktop's name in the menu bar.
Groundwork for an ADHD aid that colour-codes desktops and, later, names them
from what is running with an on-device VLM.

```
make install   # release build, replace /Applications/Spaces Labels.app, launch
make run       # debug build with latency/placement diagnostics on stderr
make probe     # print Spaces, apps per Space, and the cost of each query
cargo run --release -p app-context --bin context_probe -- --no-chrome   # per-Space context as JSON
```

The label itself needs no permissions: Spaces and window ownership come from
SkyLight's private `CGS*` connection API. The **more ▾** view (each desktop's
windows, tabs and terminals) uses:

| App | Source | Permission |
| --- | --- | --- |
| Chrome | AppleScript: tab titles and URLs, window bounds | Automation (prompted) |
| Firefox (all channels) | each profile's `recovery.jsonlz4` session store (lags ≤ 15 s; private windows are never saved) | none |
| Ghostty 1.3+ | AppleScript: windows → tabs → terminals (title, working directory) | Automation (prompted) |
| tmux | `tmux list-windows -a`, keyed by the session name that `set-titles` puts first in the terminal title | none |
| every app | window titles; also how Ghostty windows are matched | Screen Recording (menu bar → Allow window titles, then relaunch) |

Browser windows are matched to WindowServer windows by bounds, so tabs work
without Screen Recording. The latest context of every desktop is written to
`~/Library/Application Support/io.echelon.spaces-labels/context.json`.

The build is ad-hoc signed, so macOS treats each rebuild as a new app and
permissions have to be granted again after `make install`.

## How it stays instant

Each Space gets **its own overlay window**, pinned to it with
`CGSMoveWindowsToManagedSpace`. The label is part of the desktop, so it slides
in *with* the Space during the switch animation: nothing is detected or
repainted at switch time, and the visible latency is zero. (A single
all-Spaces window would show the old label until it noticed and repainted.)

Switch detection still matters for the menu-bar title and bookkeeping:

| Signal | Measured |
| --- | --- |
| `CGSGetActiveSpace` (polled every 2 ms) | < 1 µs per call; sees a switch within 2 ms |
| `NSWorkspaceActiveSpaceDidChangeNotification` | 5–16 ms after the poll |
| Full Spaces snapshot | ~100 µs |
| Apps per Space (all Spaces) | ~4–15 ms, off the main thread |

App lists refresh on app launch/quit/activate/hide notifications and on a
750 ms rescan (window opened, closed, or moved between Spaces), and are pushed
to a window only when they change.

## Layout

| Path | Responsibility |
| --- | --- |
| `crates/spaces_sys/` | SkyLight/CoreGraphics FFI: Spaces snapshot, active Space, window→Space, apps per Space, display bounds; `spaces_probe` diagnostics binary. |
| `crates/app_context/` | Per-window context: Chrome/Ghostty AppleScript (`osascript` with a timeout), Firefox session store, tmux; matching to windows. |
| `crates/spaces_labels_app/` | Tauri 2 tray app: per-Space overlay windows (`overlay.rs`), poll/notification/rescan engine (`engine.rs`), context thread (`context.rs`), settings. Overlays are click-through except where the page reports a clickable region (the "more" link or the open panel), hit-tested at ~60 Hz; a click hands focus straight back to the previous app. |
| `ui/` | Plain HTML/CSS/JS overlay. |

## Known limits

- Each overlay is a WKWebView, so N desktops cost N WebContent processes
  (about 330 MB RSS for 9, shared pages included). A native `NSTextField`
  overlay would be far lighter if that matters.
- Full-screen app Spaces get no overlay (the tray still names them).
- Desktops are numbered per display, as Mission Control shows them.
