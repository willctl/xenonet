// The tray orb (the menu bar item on macOS): how many things wait on you
// (amber), sessions and actors waiting and programs asking to be actors, or
// how many agents are working, a tooltip listing them, and a menu for the
// common jumps and the themes. What waits is the Needs you queue
// (core/needs-you.js), in its order. It polls the server's /api/summary
// every few seconds.
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
import { queueOfSummary, countOf, waitsWords } from "../core/needs-you.js";

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
    const queue = queueOfSummary(s);
    const waiting = countOf(queue);
    const parts = [`${s.working} working`];
    if (waiting) parts.push(`${waiting} need${waiting === 1 ? "s" : ""} you`);
    if (s.collisions) parts.push(`${s.collisions} collision${s.collisions === 1 ? "" : "s"}`);
    if (s.stuck.length) parts.push(`${s.stuck.length} stuck`);
    const lines = queue.slice(0, 3).map((n) => `${n.kind === "question" || n.kind === "permission" ? "◉" : "◇"} ${named(n, 40)}`);
    return [`${app.name} · ${parts.join(" · ")}`, ...lines].join("\n").slice(0, 127);
  }

  // One of the queue's items in the menu: a session opens in its app (one
  // on another machine shows on the map), an actor shows on the map, and
  // programs asking to be actors open the queue, where Allow is.
  const openQueue = () => (showWindow(), sendCommand("needs-you"));
  function queueItem(n) {
    if (n.kind === "requests") return { label: `${n.title} ${waitsWords(n)}`, click: openQueue };
    if (n.kind === "actor") return { label: `Actor · ${named(n, 40)} ${waitsWords(n)}`.slice(0, 64), click: () => (showWindow(), select(n.id)) };
    return { label: `${n.app === "codex" ? "Codex" : "Claude"} · ${named(n, 48)}`, click: () => (n.host ? (showWindow(), select(n.id)) : post("/api/open", { nodeId: n.id, action: "open-app" })) };
  }

  function menu(s) {
    const need = s ? queueOfSummary(s) : [];
    const waiting = countOf(need);
    const stuck = s?.stuck ?? [];
    return Menu.buildFromTemplate([
      { label: `Open ${app.name}`, click: () => showWindow() },
      { label: "Wallpaper mode", click: () => (showWindow(), sendCommand("wallpaper")) },
      { type: "separator" },
      {
        label: waiting ? `Needs you (${waiting})` : "Nothing needs you",
        enabled: need.length > 0,
        submenu: need.length ? [{ label: "Open the queue", click: openQueue }, { type: "separator" }, ...need.map(queueItem)] : undefined,
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
      { label: "Background service...", click: () => (showWindow(), sendCommand("background")) },
      { label: "Updates...", click: () => (showWindow(), sendCommand("updates")) },
      { label: app.isPackaged ? "Quit XenoNet Desktop" : `Quit ${app.name}`, accelerator: isMac ? "Cmd+Q" : undefined, click: () => app.quit() },
    ]);
  }

  // Rebuilt only when something in it changed, so an open menu isn't swapped
  // out from under the pointer every poll.
  function refreshMenu() {
    const key = JSON.stringify([summary ? queueOfSummary(summary).map((n) => [n.key, n.title, n.waits ?? null]) : null, summary?.stuck?.map((n) => [n.id, n.title]), opensAtLogin(), themeMenu]);
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
    await setIcon(summary?.working ?? 0, summary ? countOf(queueOfSummary(summary)) : 0);
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
