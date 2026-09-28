// Renders this window's Space label. Rust pushes "overlay-state" only when
// something changed; the initial fetch covers events sent before load.
const { invoke } = window.__TAURI__.core;
const currentWindow = window.__TAURI__.webviewWindow.getCurrentWebviewWindow();

const MAX_APPS = 8;
const label = document.getElementById("label");
const nameEl = document.getElementById("name");
const appsEl = document.getElementById("apps");

function render(state) {
  if (!state) return;
  document.body.className = state.corner;
  document.documentElement.style.setProperty("--accent", state.color);
  nameEl.textContent = state.name;

  const shown = state.apps.slice(0, MAX_APPS);
  const rows = shown.map((name) => {
    const li = document.createElement("li");
    li.textContent = name; // App names are data: never parse them as HTML.
    return li;
  });
  const hiddenCount = state.apps.length - shown.length;
  if (hiddenCount > 0) {
    const li = document.createElement("li");
    li.className = "more";
    li.textContent = `+${hiddenCount} more`;
    rows.push(li);
  }
  appsEl.replaceChildren(...rows);
  appsEl.hidden = !state.show_apps || rows.length === 0;
  label.hidden = false;
}

currentWindow.listen("overlay-state", (event) => render(event.payload));
invoke("overlay_state").then(render);
