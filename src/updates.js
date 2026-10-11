import { escapeHtml as h } from "./format.js";

export function createUpdatesPanel({ modals, desktop, toast }) {
  const api = desktop?.updates;
  let panel = null;
  let unsubscribe;
  let state = null;
  let components = [];
  let componentMessage = "";
  let componentBusy = false;
  const messages = {
    idle: "XenoNet checks for new releases in the background.",
    development: "You're running XenoNet from source. Installed copies check for releases.",
    checking: "Checking for a new release...",
    current: "You're using the latest release.",
    available: "A new version of XenoNet is available.",
    downloading: "Downloading the update...",
    ready: "Desktop is ready to restart. Your terminals keep running, and sessions stay connected to XenoNet.",
    error: "XenoNet couldn't finish the update. Check your connection and try again.",
  };
  function render(next) {
    state = next;
    if (Array.isArray(next.components)) components = next.components;
    if (next.componentError) componentMessage = next.componentError;
    if (!panel?.isConnected) return;
    panel.querySelector(".m-sub").textContent = `Version ${next.current}`;
    panel.querySelector(".m-body").innerHTML = `
      <p role="status">${h(messages[next.status] ?? messages.idle)}</p>
      ${next.version ? `<p>XenoNet ${h(next.version)}${next.status === "downloading" ? ` · ${next.progress ?? 0}%` : ""}</p>` : ""}
      ${next.status === "available" && !next.automaticInstall ? "<p>This Mac build needs a manual install. Download the new version and replace XenoNet in Applications. Your settings stay where they are.</p>" : ""}
      ${next.status !== "development" ? `<p><label><input data-update-auto type="checkbox" ${next.automaticChecks ? "checked" : ""}> Check for updates automatically${next.automaticInstall ? " and download them" : ""}</label></p>` : ""}
      ${components.length ? `<h3>Service and tools</h3><p>These update separately from Desktop. Close running terminals before installing a service update.</p>${components.map((item) => `<p>${h(item.component === "service" ? "Service" : "Tools")} ${h(item.current ?? "not installed")}${item.available ? ` → ${h(item.version)} <button class="act" data-component="${h(item.component)}" ${componentBusy ? "disabled" : ""}>Update</button>` : " · Up to date"}</p>`).join("")}` : ""}
      ${componentMessage ? `<p role="status">${h(componentMessage)}</p>` : ""}
      <div class="m-foot">
        <button class="act" data-update="check" ${["development", "checking", "downloading", "ready"].includes(next.status) ? "disabled" : ""}>Check for updates</button>
        ${next.status === "available" && next.automaticInstall ? '<button class="act primary" data-update="download">Download update</button>' : ""}
        ${next.status === "ready" ? '<button class="act primary" data-update="install">Restart and update</button>' : ""}
        <button class="act" data-update="releases">Release notes and downloads</button>
      </div>`;
  }
  async function action(name, value) {
    if (!api) return null;
    if (name === "components") { const result = await api.act(name); components = Array.isArray(result) ? result : []; if (state) render({ ...state, components }); return components; }
    if (name === "component-update") {
      componentBusy = true; componentMessage = "Installing component update..."; render(state);
      try { const result = await api.act(name, value); componentMessage = result?.detail || "Component updated."; await action("components"); return result; }
      catch (err) { componentMessage = err.message; throw err; }
      finally { componentBusy = false; render(state); }
    }
    const next = await api.act(name, value);
    if (next) render(next);
    if (name === "check" && next?.status !== "development") await action("components");
    return next;
  }
  async function open() {
    if (!api) { toast("Update the desktop app from XenoNet's GitHub releases."); return null; }
    panel = modals.open('<header class="m-head"><div><div class="m-chip">XenoNet</div><h2>Updates</h2><div class="m-sub">Loading...</div></div><button class="act" data-mclose>Close</button></header><div class="m-body"></div>', { onClose: () => { unsubscribe?.(); panel = null; } });
    panel.addEventListener("click", (event) => { const button = event.target.closest("[data-update]"); if (button) void action(button.dataset.update).catch((err) => toast(err.message)); const component = event.target.closest("[data-component]"); if (component) void action("component-update", component.dataset.component).catch((err) => toast(err.message)); });
    panel.addEventListener("change", (event) => { if (event.target.matches("[data-update-auto]")) void action("automatic", event.target.checked); });
    unsubscribe = api.onState(render);
    const next = await action("status");
    if (next?.status !== "development") await action("components").catch((err) => { componentMessage = err.message; render(state); });
    return next;
  }
  return { open, action, get state() { return state; } };
}
