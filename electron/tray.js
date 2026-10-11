// The tray orb (the menu bar item on macOS): how many agents need you (amber)
// or are working, a tooltip listing them, and a menu for the common jumps
// and the themes. It polls the server's /api/summary every few seconds.
// Notifications come from the page (alerts.js), so each alert gets exactly
// one.
//   Windows: the app icon with a numbered badge.
//   macOS:   a monochrome glyph that follows the menu bar's colors, turning
//            amber when something needs you, with the count beside it. The
//            Dock icon carries the same count while the window is open.
import { Tray, Menu, app } from "electron";
import { iconImage, trayGlyph } from "./icons.js";
import { loginSupported, loginLabel, opensAtLogin, setOpensAtLogin, onLoginChange } from "./login.js";
import { themeItems } from "./theme-menu.js";

const isMac = process.platform === "darwin";

// `themes`: the Theme menu as the page last said it (theme-menu.js), or
// null; setThemes() gives a newer one.
export function createTray({ baseUrl, showWindow, sendCommand, select, themes = null }) {
  let tray = null;
  let last = { working: -1, need: -1 };
  let lastMenu = "";
  let summary = null;
  let themeMenu = themes;

  const post = (url, body) => fetch(baseUrl + url, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) }).catch(() => {});
  const image = (working, need) => (isMac ? trayGlyph(working, need) : iconImage(32, working, need, 2));
  // A session on another machine (with XENONET_HOSTS=1) says which; it opens
  // nothing here, since hosts are read-only, so a click shows it on the map.
  const named = (n, max) => `${n.host ? `${n.host} · ` : ""}${n.title}`.slice(0, max);

  async function setIcon(working, need) {
    if (working === last.working && need === last.need) return;
    last = { working, need };
    tray.setImage(await image(working, need));
    if (isMac) {
      const n = need || working;
      tray.setTitle(n ? String(Math.min(n, 99)) : "", { fontType: "monospacedDigit" });
      app.dock?.setBadge(need ? String(need) : "");
    }
  }

  function tooltip(s) {
    if (!s) return `${app.name} · connecting`;
    const parts = [`${s.working} working`];
    if (s.needsYou.length) parts.push(`${s.needsYou.length} need${s.needsYou.length === 1 ? "s" : ""} you`);
    if (s.collisions) parts.push(`${s.collisions} collision${s.collisions === 1 ? "" : "s"}`);
    if (s.stuck.length) parts.push(`${s.stuck.length} stuck`);
    const lines = s.needsYou.slice(0, 3).map((n) => `◉ ${named(n, 40)}`);
    return [`${app.name} · ${parts.join(" · ")}`, ...lines].join("\n").slice(0, 127);
  }

  function menu(s) {
    const need = s?.needsYou ?? [];
    const stuck = s?.stuck ?? [];
    return Menu.buildFromTemplate([
      { label: `Open ${app.name}`, click: () => showWindow() },
      { label: "Wallpaper mode", click: () => (showWindow(), sendCommand("wallpaper")) },
      { type: "separator" },
      {
        label: need.length ? `Needs you (${need.length})` : "Nothing needs you",
        enabled: need.length > 0,
        // A Grok Build session has no app to open in yet: XenoNet shows it.
        submenu: need.length ? need.map((n) => ({ label: `${{ codex: "Codex", grok: "Grok Build" }[n.app] ?? "Claude"} · ${named(n, 48)}`, click: () => (n.host || n.app === "grok" ? (showWindow(), select(n.id)) : post("/api/open", { nodeId: n.id, action: "open-app" })) })) : undefined,
      },
      ...(stuck.length ? [{ label: `May be stuck (${stuck.length})`, submenu: stuck.map((n) => ({ label: named(n, 48), click: () => (showWindow(), select(n.id)) })) }] : []),
      { type: "separator" },
      { label: "Replay the last 24 hours", click: () => (showWindow(), sendCommand("replay")) },
      { label: "Show the day's log", click: () => (showWindow(), sendCommand("daylog")) },
      { label: "Open the terminal", click: () => (showWindow(), sendCommand("terminal")) },
      // Every theme, the one in use checked: a click switches it live, in
      // the window as it is, shown or not.
      ...(themeMenu ? [{ label: "Theme", submenu: themeItems(themeMenu, (t) => sendCommand(t.command), { mnemonics: !isMac }) }] : []),
      { type: "separator" },
      ...(loginSupported ? [{ label: loginLabel, type: "checkbox", checked: opensAtLogin(), click: (item) => setOpensAtLogin(item.checked) }] : []),
      { label: "Updates...", click: () => (showWindow(), sendCommand("updates")) },
      { label: app.isPackaged ? "Quit XenoNet Desktop" : `Quit ${app.name}`, accelerator: isMac ? "Cmd+Q" : undefined, click: () => app.quit() },
    ]);
  }

  // Rebuilt only when something in it changed, so an open menu isn't swapped
  // out from under the pointer every poll.
  function refreshMenu() {
    const key = JSON.stringify([summary?.needsYou?.map((n) => [n.id, n.title]), summary?.stuck?.map((n) => [n.id, n.title]), opensAtLogin(), themeMenu]);
    if (key === lastMenu) return;
    lastMenu = key;
    tray.setContextMenu(menu(summary));
  }

  async function poll() {
    try {
      summary = await fetch(baseUrl + "/api/summary").then((r) => r.json());
    } catch {
      summary = null;
    }
    const need = summary?.needsYou ?? [];
    await setIcon(summary?.working ?? 0, need.length);
    tray.setToolTip(tooltip(summary));
    refreshMenu();
  }

  async function start() {
    tray = new Tray(await image(0, 0));
    tray.setToolTip(app.name);
    onLoginChange(() => ((lastMenu = ""), refreshMenu()));
    // On macOS a click opens the menu, as menu bar items do.
    if (!isMac) tray.on("click", () => showWindow(true));
    await poll();
    setInterval(poll, 3000);
    return tray;
  }

  return {
    start,
    // The page's themes changed, or which is in use.
    setThemes(menu) {
      themeMenu = menu;
      if (tray) refreshMenu();
    },
    get summary() {
      return summary;
    },
  };
}
