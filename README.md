# Spaces Labels

A transparent, click-through label on every macOS desktop (Space) naming it and
listing the apps running there, plus the current desktop's name in the menu bar.
Groundwork for an ADHD aid that colour-codes desktops and, later, names them
from what is running with an on-device VLM.

```
make install   # release build, replace /Applications/Spaces Labels.app, launch
make run       # debug build with latency/placement diagnostics on stderr
make probe     # print Spaces, apps per Space, and the cost of each query
```

No permissions are needed: Spaces and window ownership come from SkyLight's
private `CGS*` connection API, which does not require Accessibility or Screen
Recording.

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
| `crates/spaces_labels_app/` | Tauri 2 tray app: per-Space overlay windows (`overlay.rs`), poll/notification/rescan engine (`engine.rs`), settings. |
| `ui/` | Plain HTML/CSS/JS overlay. |

## Known limits

- Each overlay is a WKWebView, so N desktops cost N WebContent processes
  (about 330 MB RSS for 9, shared pages included). A native `NSTextField`
  overlay would be far lighter if that matters.
- Full-screen app Spaces get no overlay (the tray still names them).
- Desktops are numbered per display, as Mission Control shows them.
