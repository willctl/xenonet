// The updater owns network and installer work. The page only gets status
// and named actions, never a feed URL or a path to execute.
export function createUpdates({ updater, enabled, automaticInstall, version, preferences, changed = () => {}, ready = () => {}, available = () => {}, schedule = setTimeout, cancel = clearTimeout, openRelease, confirmRestart, checkComponents }) {
  let state = { status: enabled ? "idle" : "development", current: version, version: null, automaticInstall, automaticChecks: preferences.get("automaticUpdates") !== false, checkedAt: null, progress: null };
  let timer;
  let checking = false;
  let downloading = false;
  let installing = false;
  let disposed = false;
  let announced = null;
  const listeners = [];
  const snapshot = () => ({ ...state });
  const set = (patch) => { state = { ...state, ...patch }; changed(snapshot()); };
  const on = (event, fn) => { updater.on(event, fn); listeners.push([event, fn]); };
  function later(delay = 6 * 60 * 60 * 1000) {
    cancel(timer);
    if (!disposed && enabled && state.automaticChecks) {
      timer = schedule(() => { void check(); }, delay);
      timer?.unref?.();
    }
  }
  function failed() {
    set({ status: "error", message: "XenoNet couldn't finish the update. Check your connection and try again.", progress: null });
  }
  if (enabled) {
    updater.autoDownload = false;
    // Quit can end terminal work. Only the explicit restart action installs.
    updater.autoInstallOnAppQuit = false;
    updater.allowPrerelease = false;
    updater.allowDowngrade = false;
    updater.disableWebInstaller = true;
    updater.logger = { info() {}, warn() {}, error() {}, debug() {} };
    on("error", failed);
    on("update-available", (info) => {
      set({ status: "available", version: info.version, checkedAt: Date.now(), progress: null });
      if (!automaticInstall && announced !== info.version) { announced = info.version; available(snapshot()); }
    });
    on("update-not-available", () => set({ status: "current", version: null, checkedAt: Date.now(), progress: null }));
    on("download-progress", (info) => set({ status: "downloading", progress: Math.max(0, Math.min(100, Math.round(info.percent))) }));
    on("update-downloaded", (info) => { set({ status: "ready", version: info.version, progress: 100 }); ready(snapshot()); });
  }
  async function download() {
    if (!enabled || !automaticInstall || downloading || state.status !== "available") return snapshot();
    downloading = true;
    set({ status: "downloading", progress: 0 });
    try { await updater.downloadUpdate(); } catch { failed(); }
    finally { downloading = false; }
    return snapshot();
  }
  async function check() {
    if (!enabled || checking || downloading || state.status === "ready") return snapshot();
    checking = true;
    set({ status: "checking", message: null, progress: null });
    try {
      await updater.checkForUpdates();
      if (checkComponents) { try { set({ components: await checkComponents(), componentError: null }); } catch (err) { set({ componentError: err.message }); } }
      if (automaticInstall && state.automaticChecks && state.status === "available") await download();
    } catch { failed(); }
    finally { checking = false; later(); }
    return snapshot();
  }
  async function act(action, value) {
    if (action === "status") return snapshot();
    if (action === "check") return check();
    if (action === "download") return download();
    if (action === "releases") { await openRelease(); return snapshot(); }
    if (action === "automatic" && typeof value === "boolean") {
      preferences.set("automaticUpdates", value);
      set({ automaticChecks: value });
      later(value ? 1000 : undefined);
      return snapshot();
    }
    if (action === "install" && enabled && automaticInstall && state.status === "ready" && !installing) {
      installing = true;
      try { if (await confirmRestart()) updater.quitAndInstall(false, true); }
      catch { failed(); }
      finally { installing = false; }
      return snapshot();
    }
    return snapshot();
  }
  return { snapshot, act, start: () => later(30000), dispose() { disposed = true; cancel(timer); for (const [event, fn] of listeners) updater.removeListener(event, fn); } };
}
