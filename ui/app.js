// Renders this window's Space label. Rust pushes "overlay-state" only when
// something changed; the initial fetch covers events sent before load.
//
// The window ignores the mouse except over the region this page reports with
// set_hit_rect (the "more" link, or the whole panel while expanded), so the
// label never blocks clicks meant for the apps beneath it.
const { invoke } = window.__TAURI__.core;
const currentWindow = window.__TAURI__.webviewWindow.getCurrentWebviewWindow();

const MAX_APPS = 8;
const MAX_TABS = 40;
const label = document.getElementById("label");
const nameEl = document.getElementById("name");
const appsEl = document.getElementById("apps");
const detailsEl = document.getElementById("details");
const moreEl = document.getElementById("more");
const focusEl = document.getElementById("focus");

let state = null;
let expanded = false;

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = text; // Titles are data: never HTML.
  return node;
}

function renderApps() {
  const shown = state.apps.slice(0, MAX_APPS);
  const rows = shown.map((name) => el("li", null, name));
  const hiddenCount = state.apps.length - shown.length;
  if (hiddenCount > 0) rows.push(el("li", "more-apps", `+${hiddenCount} more`));
  appsEl.replaceChildren(...rows);
  appsEl.hidden = expanded || !state.show_apps || rows.length === 0;
}

// "artcraft · Claude working · dev site :4201": what this desktop is for.
function projectLine(project) {
  return [project.name, ...project.signals.slice(0, 2)].join(" · ");
}

function renderFocus() {
  const projects = state.context ? state.context.projects : [];
  focusEl.textContent = projects.slice(0, 2).map(projectLine).join("   ");
  focusEl.hidden = expanded || projects.length === 0;
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
  if (!context) {
    nodes.push(el("p", "hint", "Gathering details…"));
  } else if (!context.apps.length) {
    nodes.push(el("p", "hint", "No windows on this desktop."));
  }
  for (const app of context ? context.apps : []) {
    const section = el("section", "app");
    section.append(el("h2", null, app.name));
    app.windows.forEach((win, i) => {
      // Several windows need a divider even when titles are unreadable.
      const title = win.title || (app.windows.length > 1 ? `Window ${i + 1}` : null);
      if (title && (app.windows.length > 1 || !win.tabs.length)) {
        section.append(el("div", "window-title", title));
      }
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

function render(next) {
  if (next) state = next;
  if (!state) return;
  document.body.className = state.placement + (expanded ? " expanded" : "");
  document.documentElement.style.setProperty("--accent", state.color);
  nameEl.textContent = state.name;
  renderApps();
  renderFocus();
  if (expanded) renderDetails();
  else detailsEl.hidden = true;
  moreEl.textContent = expanded ? "less ▴" : "more ▾";
  label.hidden = false;
  reportHitRect();
}

// Only report changes: the region is hit-tested at ~60 Hz on the Rust side.
let lastHit = "";
function reportHitRect() {
  const target = expanded ? label : moreEl;
  const r = target.getBoundingClientRect();
  const pad = expanded ? 0 : 6;
  const rect = r.width && r.height
    ? { x: r.left - pad, y: r.top - pad, width: r.width + 2 * pad, height: r.height + 2 * pad }
    : null;
  const key = JSON.stringify(rect);
  if (key === lastHit) return;
  lastHit = key;
  invoke("set_hit_rect", { rect });
}

moreEl.addEventListener("click", () => {
  expanded = !expanded;
  render();
  detailsEl.scrollTop = 0;
  invoke("set_expanded", { expanded });
});

new ResizeObserver(reportHitRect).observe(label);
window.addEventListener("resize", reportHitRect);
currentWindow.listen("overlay-state", (event) => render(event.payload));
invoke("overlay_state").then(render);
