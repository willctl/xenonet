// What the Windows build gets from its installer and the taskbar, done the
// macOS way: the menu bar, moving into Applications, asking once about
// opening at login, and uninstalling (macOS has no uninstaller, so XenoNet
// carries its own, with the same keep-or-remove-my-data choice).
import { app, dialog, Menu, shell } from "electron";
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { appDir } from "../core/dirs.js";
import { opensAtLogin, setOpensAtLogin, onLoginChange } from "./login.js";
import { prefs } from "./prefs.js";
import { themeItems } from "./theme-menu.js";

const REPO = "https://github.com/willctl/xenonet";
const APP_ID = "dev.xenonet.app";

// The menu bar. Shortcuts the page handles itself (⌘K, ⌃`, W, R, M) are
// shown but not registered, so the page keeps them, the palette's ⌘O/⌘P/⌘H
// included. View has a Theme submenu once the page has said which themes
// there are (setThemes, with theme-menu.js's menu).
export function installMenu({ send, dev, uninstall }) {
  let themes = null;
  const page = (label, cmd, accelerator) => ({ label, click: () => send(cmd), ...(accelerator ? { accelerator, registerAccelerator: false } : {}) });
  const build = () =>
    Menu.setApplicationMenu(
      Menu.buildFromTemplate([
        {
          label: app.name,
          submenu: [
            { role: "about" },
            { label: "Updates...", click: () => send("updates") },
            { type: "separator" },
            { label: "Open at Login", type: "checkbox", checked: opensAtLogin(), click: (item) => setOpensAtLogin(item.checked) },
            { label: "Background Service…", click: () => send("background") },
            { type: "separator" },
            { role: "services" },
            { type: "separator" },
            { role: "hide" },
            { role: "hideOthers" },
            { role: "unhide" },
            { type: "separator" },
            { label: "Uninstall XenoNet…", enabled: app.isPackaged, click: () => uninstall() },
            { role: "quit", label: app.isPackaged ? "Quit XenoNet Desktop" : `Quit ${app.name}` },
          ],
        },
        { role: "editMenu" },
        {
          label: "View",
          submenu: [
            page("Command Palette", "palette", "Cmd+K"),
            page("Terminal", "terminal", "Ctrl+`"),
            page("Wallpaper Mode", "wallpaper", "W"),
            page("Reset View", "reset", "R"),
            page("Mute or Unmute Sound", "mute", "M"),
            // Every theme, the one in use checked, switching live.
            ...(themes ? [{ label: "Theme", submenu: themeItems(themes, (t) => send(t.command)) }] : []),
            { type: "separator" },
            page("Replay the Last 24 Hours", "replay"),
            page("Show the Day's Log", "daylog"),
            page("Make a Poster of Today", "poster"),
            { type: "separator" },
            { role: "togglefullscreen" },
            { type: "separator" },
            ...(dev ? [{ role: "reload" }, { role: "forceReload" }] : []),
            { role: "toggleDevTools" },
          ],
        },
        {
          role: "windowMenu",
          submenu: [
            // The page decides: the focused terminal tab, or else the window.
            { label: "Close", accelerator: "Cmd+W", click: () => send("close") },
            { role: "minimize" },
            { role: "zoom" },
            { type: "separator" },
            { role: "front" },
          ],
        },
        {
          role: "help",
          submenu: [
            { label: "XenoNet on GitHub", click: () => shell.openExternal(REPO) },
            { label: "Report an Issue", click: () => shell.openExternal(`${REPO}/issues`) },
          ],
        },
      ]),
    );
  build();
  onLoginChange(build);
  return {
    // The page's themes changed, or which is in use.
    setThemes(menu) {
      themes = menu;
      build();
    },
  };
}

export const bundlePath = () => path.resolve(path.dirname(process.execPath), "..", "..");

// A copy started from the disk image or Downloads offers to move itself into
// Applications, where it survives ejecting and can open at login. Returns
// true when XenoNet is relaunching from there.
export async function offerMove() {
  const p = prefs();
  if (!app.isPackaged || app.isInApplicationsFolder() || p.get("skipMove")) return false;
  const bundle = bundlePath();
  const where = bundle.startsWith("/Volumes/") ? "the disk image" : path.dirname(bundle).replace(os.homedir(), "~");
  const { response, checkboxChecked } = await dialog.showMessageBox({
    type: "question",
    message: "Move XenoNet to the Applications folder?",
    detail: `XenoNet is running from ${where}. In Applications it keeps working after you eject the disk image or tidy Downloads, and it can open at login.`,
    buttons: ["Move to Applications", "Not Now"],
    defaultId: 0,
    cancelId: 1,
    checkboxLabel: "Don't ask again",
  });
  if (response !== 0) {
    if (checkboxChecked) p.set("skipMove", true);
    return false;
  }
  try {
    return app.moveToApplicationsFolder();
  } catch (err) {
    dialog.showErrorBox("XenoNet couldn't move itself", `${err.message ?? err}\n\nDrag XenoNet into Applications in Finder instead.`);
    return false;
  }
}

// The Windows installer asks this on its Options page; on a Mac the first
// launch does, once.
export async function askAboutLogin(win) {
  const p = prefs();
  if (!app.isPackaged || p.get("askedLogin")) return;
  p.set("askedLogin", true);
  if (opensAtLogin()) return;
  const { response } = await dialog.showMessageBox(win, {
    type: "question",
    message: "Open XenoNet when you log in?",
    detail: "XenoNet waits in the menu bar and tells you when a session needs you, even with its window closed. You can change this any time from the menu bar icon or the XenoNet menu.",
    buttons: ["Open at Login", "Not Now"],
    defaultId: 0,
    cancelId: 1,
  });
  if (response === 0) await setOpensAtLogin(true);
}

// Everything XenoNet writes, for "also remove my data": the places the Windows
// uninstaller's /PURGE clears, in their macOS homes. Posters and clips in
// Pictures and Movies are kept, as on Windows.
export function dataDirs() {
  const lib = path.join(os.homedir(), "Library");
  const dirs = [
    appDir("state"), // layout and dismissed alerts (or $XDG_STATE_HOME/xenonet)
    appDir("cache"), // the search index (or $XDG_CACHE_HOME/xenonet)
    app.getPath("userData"), // the window's own settings: mute, terminal height
    path.join(lib, "Logs", "XenoNet"),
    path.join(lib, "Caches", APP_ID),
    path.join(lib, "HTTPStorages", APP_ID),
    path.join(lib, "Saved Application State", `${APP_ID}.savedState`),
    path.join(lib, "Preferences", `${APP_ID}.plist`),
  ];
  // Only ever XenoNet's own folders, whatever the environment says.
  return [...new Set(dirs)].filter((d) => /xenonet/i.test(path.basename(d)) && fs.existsSync(d));
}

// Data is removed by a small shell that waits for XenoNet to exit, so nothing
// is written back on the way out.
function removeAfterExit(dirs) {
  if (!dirs.length) return;
  const script = 'while kill -0 "$1" 2>/dev/null; do sleep 0.2; done; shift; rm -rf -- "$@"';
  spawn("/bin/sh", ["-c", script, "sh", String(process.pid), ...dirs], { detached: true, stdio: "ignore" }).unref();
}

// Uninstall: stop opening at login, move XenoNet.app to the Trash, optionally
// remove its data, and quit. `ask: false` is the scripted form
// (`XenoNet --uninstall [--purge]`).
export async function uninstall({ ask = true, purge = false, win = null, stop = () => {} } = {}) {
  const bundle = bundlePath();
  if (!app.isPackaged || !bundle.endsWith(".app")) {
    if (ask) dialog.showErrorBox("Nothing to uninstall", "This copy of XenoNet runs from source. Delete the folder to remove it.");
    else console.error("[xenonet] --uninstall works on the installed app only");
    return false;
  }
  if (ask) {
    const { response, checkboxChecked } = await dialog.showMessageBox(win, {
      type: "warning",
      message: "Uninstall XenoNet Desktop?",
      detail: "Desktop moves to the Trash and stops opening at login. The service and tools remain installed. Removing XenoNet data also stops the service and its terminals. Your Claude Code and Codex sessions, posters and clips are left alone.",
      checkboxLabel: "Also remove my XenoNet data (saved layout, dismissed alerts, search index and settings)",
      buttons: ["Uninstall", "Cancel"],
      defaultId: 1,
      cancelId: 1,
    });
    if (response !== 0) return false;
    purge = checkboxChecked;
  }
  // What a purge runs lives in the bundle that's about to go to the Trash,
  // so it's loaded now. Loaded after the move, the imports failed and the
  // uninstall stopped halfway, with the hooks, the service and the data
  // still in place.
  const purging = purge ? await Promise.all([import("../core/engine/connect.js"), import("../core/components/runtime.js")]).catch((err) => (console.error(`[xenonet] couldn't load what a purge needs: ${err.message ?? err}`), null)) : null;
  // The login item goes first, while the bundle it points at is still there.
  const atLogin = opensAtLogin();
  if (atLogin) app.setLoginItemSettings({ openAtLogin: false });
  try {
    await shell.trashItem(bundle);
  } catch (err) {
    if (atLogin) app.setLoginItemSettings({ openAtLogin: true });
    const why = `${err.message ?? err}\n\nQuit XenoNet, then drag it from ${path.dirname(bundle)} to the Trash.`;
    if (ask) dialog.showErrorBox("XenoNet couldn't move itself to the Trash", why);
    else console.error(`[xenonet] couldn't move ${bundle} to the Trash: ${why}`);
    return false;
  }
  if (purge) {
    // A step that fails is said, and the rest still run: the data goes, and
    // XenoNet quits.
    const [connect, runtime] = purging ?? [];
    // XenoNet's hooks and MCP servers in Claude Code and Codex, the console's
    // too, run files about to be deleted: take them out first.
    try {
      await connect.createConnect({ port: 47840, root: app.getAppPath(), consoleMcp: true }).remove();
    } catch (err) {
      console.error(`[xenonet] couldn't take XenoNet out of Claude Code and Codex: ${err.message ?? err}`);
    }
    try {
      const client = await runtime.connectService().catch(() => null);
      if (client) { try { await client.request("stop", { force: true }); } finally { client.close(); } }
    } catch (err) {
      console.error(`[xenonet] couldn't stop the service: ${err.message ?? err}`);
    }
    removeAfterExit(dataDirs());
  }
  stop();
  app.exit(0);
  return true;
}
