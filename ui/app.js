// Renders this window's Space label. Rust pushes "overlay-state" only when
// something changed; the initial fetch covers events sent before load.
//
// The window ignores the mouse except over the region this page reports with
// set_hit_rect (the "more" link, or the whole panel while expanded), so the
// label never blocks clicks meant for the apps beneath it.
const { invoke } = window.__TAURI__.core;
const currentWindow = window.__TAURI__.webviewWindow.getCurrentWebviewWindow();

const MAX_CHIPS = 7;
const MAX_TABS = 40;
const $ = (id) => document.getElementById(id);
const label = $("label");
const eyebrowEl = $("eyebrow");
const nameEl = $("name");
const subtitleEl = $("subtitle");
const summaryEl = $("summary");
const chipsEl = $("chips");
const detailsEl = $("details");
const moreEl = $("more");
const renameOpen = $("rename-open");
const renameForm = $("rename");
const renameName = $("rename-name");
const renameDesc = $("rename-desc");

let state = null;
let expanded = false;
let editing = false;

// Show-then-fade. Each desktop has its own overlay window, which slides in
// with its desktop, so a label must be fully shown ("shown") before its
// desktop appears. On arrival it waits `show_ms`, fades over `fade_ms`
// ("fading") and ends "hidden" (invisible and click-through). Hovering a
// shown or fading label holds it; hovering a hidden one does nothing. After
// the desktop is left (and has slid away, `rearm_ms`) the label is reset to
// "shown", ready for the next visit.
let phase = "shown";
let showing = false;
let pointerInside = false;
let ctrlHeld = false;
let fadeTimer = 0;
let fadePending = false;
let rearmTimer = 0;
const DEFAULT_TIMING = { show_ms: 1100, fade_ms: 700, rearm_ms: 600, linger_ms: 400, panel_opacity: 0.85 };
const timing = () => (state && state.timing) || DEFAULT_TIMING;
const autoHide = () => state && state.auto_hide && !expanded && !editing;

function setPhase(next, durationMs) {
  label.style.transitionDuration = `${durationMs}ms`;
  phase = next;
  document.body.dataset.phase = next;
  reportHitRect();
}

function clearTimers() {
  clearTimeout(fadeTimer);
  clearTimeout(rearmTimer);
  fadePending = false;
}

// Fade after `delay` unless something holds the label by then.
function scheduleFade(delay) {
  clearTimeout(fadeTimer);
  fadePending = false;
  if (!autoHide()) return;
  fadePending = true;
  fadeTimer = setTimeout(() => {
    if (pointerInside || ctrlHeld || !autoHide()) {
      fadePending = false;
      return;
    }
    setPhase("fading", timing().fade_ms);
    fadeTimer = setTimeout(() => {
      fadePending = false;
      setPhase("hidden", 0);
    }, timing().fade_ms);
  }, delay);
}

// Failsafe: a label that is up, on screen and held by nothing always gets a
// fade scheduled, whatever event might have been lost.
setInterval(() => {
  if (showing && autoHide() && phase === "shown" && !pointerInside && !ctrlHeld && !fadePending) {
    scheduleFade(timing().linger_ms);
  }
}, 500);

// Clicking the title or the desktop line dismisses the label at once (the
// app chips stay click-through), and hands focus back to the previous app.
function dismiss() {
  if (phase === "hidden") return;
  if (editing) closeRename();
  if (expanded) {
    expanded = false;
    render();
    invoke("set_expanded", { expanded: false });
  }
  clearTimers();
  setPhase("hidden", 120);
  invoke("dismissed");
}

function rearm() {
  clearTimers();
  if (editing) closeRename();
  if (expanded) {
    expanded = false;
    invoke("set_expanded", { expanded: false });
  }
  setPhase("shown", 0);
  render();
}

function setShowing(next) {
  if (next === showing) return;
  showing = next;
  clearTimers();
  if (showing) {
    if (phase !== "shown") rearm(); // Came back before the reset ran.
    scheduleFade(timing().show_ms);
  } else {
    pointerInside = false;
    rearmTimer = setTimeout(rearm, timing().rearm_ms);
  }
}

// Control held (see Rust `update_hover`): hold a label that is still up;
// release fades it like the pointer leaving. A gone label ignores it.
function setHold(held) {
  // Rust re-sends the state twice a second; act only on changes.
  if (held === ctrlHeld) return;
  ctrlHeld = held;
  if (phase === "hidden" || !showing) return;
  if (held) {
    clearTimeout(fadeTimer);
    if (phase === "fading") setPhase("shown", 150);
  } else if (!pointerInside) {
    scheduleFade(timing().linger_ms);
  }
}

// Menu bar → Show Label: back to full opacity, then fade as on arrival.
function reveal() {
  if (!showing) return;
  rearm();
  scheduleFade(timing().show_ms);
}

function setPointer(inside) {
  if (inside === pointerInside) return;
  pointerInside = inside;
  if (phase === "hidden" || !showing) return;
  if (inside) {
    clearTimeout(fadeTimer);
    setPhase("shown", 150);
  } else if (!ctrlHeld) {
    scheduleFade(timing().linger_ms);
  }
}

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = text; // Titles are data: never HTML.
  return node;
}

// Agents have no app icon; draw simple marks for them.
const SVG = "http://www.w3.org/2000/svg";
const AGENT_ICONS = {
  // A radiating burst.
  claude: (svg) => {
    for (let i = 0; i < 8; i += 1) {
      const ray = document.createElementNS(SVG, "rect");
      ray.setAttribute("x", "10.8");
      ray.setAttribute("y", "2");
      ray.setAttribute("width", "2.4");
      ray.setAttribute("height", "9");
      ray.setAttribute("rx", "1.2");
      ray.setAttribute("fill", "#e07a52");
      ray.setAttribute("transform", `rotate(${i * 45} 12 12)`);
      svg.append(ray);
    }
  },
  // A prompt ">_" in a rounded square.
  codex: (svg) => {
    const box = document.createElementNS(SVG, "rect");
    box.setAttribute("x", "2");
    box.setAttribute("y", "2");
    box.setAttribute("width", "20");
    box.setAttribute("height", "20");
    box.setAttribute("rx", "6");
    box.setAttribute("fill", "#f1f3f5");
    const prompt = document.createElementNS(SVG, "path");
    prompt.setAttribute("d", "M7 8l4 4-4 4M12.5 16.5h4.5");
    prompt.setAttribute("stroke", "#111");
    prompt.setAttribute("stroke-width", "2");
    prompt.setAttribute("fill", "none");
    prompt.setAttribute("stroke-linecap", "round");
    prompt.setAttribute("stroke-linejoin", "round");
    svg.append(box, prompt);
  },
};

function chipIcon(chip) {
  if (chip.agent && AGENT_ICONS[chip.agent]) {
    const svg = document.createElementNS(SVG, "svg");
    svg.setAttribute("viewBox", "0 0 24 24");
    svg.setAttribute("class", "icon");
    AGENT_ICONS[chip.agent](svg);
    return svg;
  }
  const url = state.icons[chip.label];
  if (!url) return el("span", "icon placeholder");
  const img = el("img", "icon");
  img.src = url;
  img.alt = "";
  return img;
}

function renderChips() {
  // Before the first context pass, show plain app names.
  const chips = state.chips.length
    ? state.chips
    : state.apps.map((name) => ({ label: name, category: "other", agent: null, state: null }));
  const shown = chips.slice(0, MAX_CHIPS);
  const rows = shown.map((chip) => {
    const li = el("li", `chip cat-${chip.category}${chip.state ? ` agent-${chip.state}` : ""}`);
    li.append(chipIcon(chip), el("span", "chip-label", chip.label));
    if (chip.state) li.append(el("span", "chip-state", chip.state));
    return li;
  });
  if (chips.length > shown.length) rows.push(el("li", "chip chip-more", `+${chips.length - shown.length}`));
  chipsEl.replaceChildren(...rows);
  chipsEl.hidden = expanded || !state.show_apps || rows.length === 0;
}

function statusClass(status) {
  if (status.includes("working")) return "status working";
  if (status.includes("waiting")) return "status waiting";
  return "status";
}

function entryRow(entry) {
  const li = el("li", entry.active ? "entry active" : "entry");
  const line = el("div", "line");
  line.append(el("span", "title", entry.title || "Untitled"));
  if (entry.status) line.append(el("span", statusClass(entry.status), entry.status));
  if (entry.detail) line.append(el("span", "detail", entry.detail));
  li.append(line);
  if (entry.note) li.append(el("div", "note", entry.note));
  if (entry.children.length) {
    const children = el("ul", "children");
    children.append(...entry.children.map(entryRow));
    li.append(children);
  }
  return li;
}

function renderDetails() {
  const nodes = [];
  if (!state.titles_readable) {
    nodes.push(el("p", "hint", "Window titles need Screen Recording: menu bar icon → Allow window titles, then relaunch."));
  }
  const vision = state.vision;
  if (vision.enabled && vision.blocked) nodes.push(el("p", "hint", `Window descriptions: ${vision.blocked}.`));
  const context = state.context;
  if (context && context.projects.length) {
    const section = el("section", "app projects");
    section.append(el("h2", null, "Projects"));
    const list = el("ul", "tabs");
    list.append(...context.projects.map((p) => entryRow({
      title: p.name, detail: p.path, active: false, status: null, note: p.signals.join(" · "), children: [],
    })));
    section.append(list);
    nodes.push(section);
  }
  if (!context) nodes.push(el("p", "hint", "Gathering details…"));
  else if (!context.apps.length) nodes.push(el("p", "hint", "No windows on this desktop."));
  const categories = Object.fromEntries(state.chips.map((c) => [c.label, c.category]));
  for (const app of context ? context.apps : []) {
    const section = el("section", `app cat-${categories[app.name] || "other"}`);
    const heading = el("h2");
    if (state.icons[app.name]) heading.append(chipIcon({ label: app.name }));
    heading.append(document.createTextNode(app.name));
    section.append(heading);
    app.windows.forEach((win, i) => {
      // Several windows need a divider even when titles are unreadable.
      const title = win.title || (app.windows.length > 1 ? `Window ${i + 1}` : null);
      if (title && (app.windows.length > 1 || !win.tabs.length)) section.append(el("div", "window-title", title));
      if (win.vision) section.append(el("div", "vision", `👁 ${win.vision}`));
      if (!win.tabs.length) return;
      const list = el("ul", "tabs");
      list.append(...win.tabs.slice(0, MAX_TABS).map(entryRow));
      if (win.tabs.length > MAX_TABS) list.append(el("li", "entry overflow", `+${win.tabs.length - MAX_TABS} more tabs`));
      section.append(list);
    });
    nodes.push(section);
  }
  detailsEl.replaceChildren(...nodes);
  detailsEl.hidden = !expanded;
}

// The title must always fit on one line: shrink it (binary search on the
// font size) down to 40% of the placement's size; only a title that still
// does not fit wraps, and it is never cut off with an ellipsis.
function fitTitle() {
  const max = parseFloat(getComputedStyle(label).getPropertyValue("--title-max")) || 34;
  const fits = (size) => {
    nameEl.style.fontSize = `${size}px`;
    return nameEl.scrollWidth <= nameEl.clientWidth + 1;
  };
  nameEl.classList.remove("wrap");
  if (fits(max)) return;
  let lo = Math.floor(max * 0.4);
  let hi = max;
  if (!fits(lo)) {
    nameEl.classList.add("wrap");
    nameEl.style.fontSize = `${lo}px`;
    return;
  }
  while (hi - lo > 1) {
    const mid = Math.floor((lo + hi) / 2);
    if (fits(mid)) lo = mid;
    else hi = mid;
  }
  nameEl.style.fontSize = `${lo}px`;
}

function render(next) {
  if (next) state = next;
  if (!state) return;
  document.body.className = state.placement + (expanded ? " expanded" : "");
  document.documentElement.style.setProperty("--accent", state.color);
  document.documentElement.style.setProperty("--panel", `rgba(10, 12, 22, ${timing().panel_opacity})`);
  label.classList.toggle("empty", state.empty);
  eyebrowEl.textContent = state.name_source === "user" ? `${state.desktop} · your name` : state.desktop;
  // An empty desktop's title already is "Desktop N".
  eyebrowEl.hidden = state.empty && !expanded;
  nameEl.textContent = state.name;
  subtitleEl.textContent = state.subtitle || "";
  subtitleEl.hidden = !state.subtitle || editing;
  const summary = state.user_description || state.ai_summary;
  summaryEl.textContent = summary || "";
  summaryEl.hidden = !expanded || !summary || editing || summary === state.subtitle;
  renameOpen.hidden = !expanded || editing;
  renameForm.hidden = !editing;
  renderChips();
  if (expanded) renderDetails();
  else detailsEl.hidden = true;
  moreEl.textContent = expanded ? "less ▴" : "more ▾";
  moreEl.hidden = state.empty && !expanded;
  label.hidden = false;
  fitTitle();
  reportHitRect();
}

// Only report changes: the region is hit-tested at ~60 Hz on the Rust side.
let lastHit = "";
function reportHitRect() {
  const box = (node, pad) => {
    const r = node.getBoundingClientRect();
    return r.width && r.height
      ? { x: r.left - pad, y: r.top - pad, width: r.width + 2 * pad, height: r.height + 2 * pad }
      : null;
  };
  // A hidden label neither takes clicks nor reacts to hovering. A shown one
  // takes clicks only on the "more" link, the title and the desktop line.
  const hidden = phase === "hidden";
  const targets = hidden ? [] : expanded ? [box(label, 0)] : [box(moreEl, 6), box(nameEl, 0), box(eyebrowEl, 4)];
  const rects = targets.filter(Boolean);
  const panel = hidden ? null : box(label, 0);
  const key = JSON.stringify([rects, panel]);
  if (key === lastHit) return;
  lastHit = key;
  invoke("set_hit_rect", { rects, panel });
}

function openRename() {
  editing = true;
  renameName.value = state.name_source === "user" ? state.name : "";
  renameName.placeholder = state.name;
  renameDesc.value = state.user_description || "";
  renameDesc.placeholder = state.subtitle || state.ai_summary || "e.g. Shipping the ArtCraft video models";
  render();
  invoke("set_editing", { editing: true }).then(() => renameName.focus());
}

function closeRename() {
  editing = false;
  render();
  invoke("set_editing", { editing: false });
}

// An empty name hands the desktop back to the automatic name.
function saveRename(name) {
  editing = false;
  render();
  invoke("rename_space", { name, description: renameDesc.value });
}

renameOpen.addEventListener("click", openRename);
nameEl.addEventListener("click", dismiss);
eyebrowEl.addEventListener("click", dismiss);
$("rename-cancel").addEventListener("click", closeRename);
$("rename-reset").addEventListener("click", () => saveRename(""));
renameForm.addEventListener("submit", (event) => {
  event.preventDefault();
  saveRename(renameName.value);
});
renameForm.addEventListener("keydown", (event) => {
  if (event.key === "Escape") closeRename();
});

moreEl.addEventListener("click", () => {
  if (editing) closeRename();
  expanded = !expanded;
  render();
  detailsEl.scrollTop = 0;
  invoke("set_expanded", { expanded });
  // An open panel stays; closing it lets the label fade again.
  if (expanded) clearTimeout(fadeTimer);
  else scheduleFade(timing().linger_ms);
});

new ResizeObserver(() => {
  fitTitle();
  reportHitRect();
}).observe(document.body);
function onState(next) {
  if (!next) return;
  const first = !state;
  const hadAutoHide = state && state.auto_hide;
  render(next);
  if (hadAutoHide && !next.auto_hide) rearm();
  // Only the first state says whether we are on screen: later pushes are
  // computed a moment earlier and can arrive after a newer "space-active",
  // which is the authority for switches.
  if (first) setShowing(next.showing);
  if (!hadAutoHide && next.auto_hide && showing && phase === "shown") scheduleFade(timing().show_ms);
}

currentWindow.listen("overlay-state", (event) => onState(event.payload));
// Pushed by the poll the moment the desktop switches (ahead of the state).
currentWindow.listen("space-active", (event) => setShowing(event.payload));
currentWindow.listen("pointer", (event) => setPointer(event.payload));
currentWindow.listen("hold", (event) => setHold(event.payload));
currentWindow.listen("reveal", reveal);
invoke("overlay_state").then(onState);
