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

| Claude Code, Codex | process tree finds the agent's tmux pane; `tmux capture-pane` gives working / waiting / idle, the live status line, and Claude's recap or Codex's task | none |
| local dev sites | `lsof` maps a `localhost:PORT` tab to the listening process and its working directory | none |
| window screenshots → local VLM | `screencapture -l` (works across Spaces) → Qwen3-VL-2B via Homebrew `llama-server` on 127.0.0.1 | Screen Recording |

Minimized windows count as absent everywhere (apps, chips, titles, naming,
screenshots): WindowServer still reports them on their Space, so they are
recognized by their "minimized" window tag. A desktop with no windows left
shows only a small "Desktop N (empty)".

Each desktop also gets a **projects** list (the repository roots its agents,
terminals, dev sites and IDE windows point at, most active first), shown under
the app list, e.g. "artcraft · Claude working · dev site :4201".

**Title and chips** (`headline.rs`, deterministic): a desktop with a project
is titled "<Project> (<tools>)", e.g. "ArtCraft Services (Claude)" or
"ArtCraft Desktop (RustRover)"; display names come from `project_names` in
`settings.json`, else the directory name title-cased. Without a project it is
titled by use: Communications, Fun, Research, GitHub, Notes, Coding (browsers
are classified by their tabs' sites, active tabs weighted most). The title
shrinks to fit on one line and never truncates. Chips show agents (with
working/waiting/idle) and apps (real macOS icons), outlined by category.

**Naming.** The same local model names each desktop from a compact digest of
all of the above (`naming.rs`); it supplies the subtitle. When a project is known the name is
"<project>: <task>" and the model only writes the task (small models otherwise
drift to app names or the agent's latest step); otherwise it names the topic.
Names are cached by desktop UUID in `names.json` and are sticky: a new desktop
is named at once, a changed project set renames it (at most every 30 s), and
anything else only after 10 minutes. **✎ Rename** in the expanded panel sets
your own name and description, which win over the model's; each rename is
appended to `feedback.jsonl` with the exact model input and the model's
suggestion, as eval data.

The VLM is optional (menu bar → Describe windows with local AI). Put
`Qwen3VL-2B-Instruct-Q8_0.gguf` and `mmproj-Qwen3VL-2B-Instruct-Q8_0.gguf`
(from `Qwen/Qwen3-VL-2B-Instruct-GGUF`) in
`~/Library/Application Support/io.echelon.spaces-labels/models/`. On an M4
Pro it takes 1.0–1.6 s per window at 1024 image tokens, near-zero CPU (Metal),
and ~3.7 GB while loaded. It runs one window at a time with a 3 s cooldown,
refreshes a window every 60 s (showing desktop) or 5 min (others) unless its
title changes, pauses under serious thermal pressure or Low Power Mode, and
stops after 10 idle minutes.

Browser windows are matched to WindowServer windows by bounds, so tabs work
without Screen Recording. The latest context of every desktop is written to
`~/Library/Application Support/io.echelon.spaces-labels/context.json`.

Builds are signed with a local self-signed identity, "Spaces Labels Dev"
(`bundle.macOS.signingIdentity`), so macOS keeps Screen Recording and
Automation grants across rebuilds. On another machine, create one (Keychain
Access → Certificate Assistant → Create a Certificate, type Code Signing) or
remove `signingIdentity` to fall back to ad-hoc signing, which loses the
grants on every rebuild.

## Show, then fade

Arriving on a desktop shows its label at full opacity; after `show_ms` it
fades (lift, shrink, blur) over `fade_ms` and is gone, click-through. Hovering
a shown or fading label holds it (it fades `linger_ms` after the pointer
leaves); hovering a gone label does nothing. `rearm_ms` after you leave a
desktop (once it has slid away) its label is reset, so it is already visible
the next time that desktop slides in. An open "more" panel or rename form
holds the label, and so does holding **Control** (e.g. pausing between
Ctrl+arrow switches; read from the session's modifier state, no permission
needed), but only while the label is still up. Menu bar → **Show Label**
brings the current desktop's label back; **Show label briefly when switching**
turns the fading off.

Overlays draw at window level 1500 (assistive-technology high): above every
app window, other always-on-top tools, menus and screen savers.

Menu bar → **Edit Settings…** opens `settings.json`; saving it applies changes
live: `show_ms` (1100), `fade_ms` (700), `rearm_ms` (600), `linger_ms` (400),
`panel_opacity` (0.85), `hold_with_ctrl` (true), `placement`, `project_names`, `auto_hide`,
`show_apps`, `vision`.

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
