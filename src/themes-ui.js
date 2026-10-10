// Themes from the console (core/themes.js for what they are, theme.js for
// drawing them): a palette command for each; "Theme…", a switcher that
// shows each theme on the map as you move through it (theme-switcher.js);
// a gallery of them all (theme-gallery.js); making a theme of your own;
// switching, which every window follows since the server keeps the choice;
// looking at one in this window before keeping it; and the desktop
// window's own colors, and its Theme menus in the tray and the menu bar.
//
// The switcher and the gallery load the first time either is opened: at
// startup they were among the page's requests, and the page waited for
// them.
import { THEME, onTheme, previewTheme, revertTheme, useTheme, themeDrawn, themeState, themeFonts } from "./theme.js";
import { tildePath } from "./format.js";

// Commands of their own; a theme of yours named like one keeps a command
// too, under another id.
const OWN = new Set(["copy", "switcher", "gallery"]);
const commandId = (id) => (OWN.has(id) ? `theme-use-${id}` : `theme-${id}`);

export function createThemes({ post, sfx, toast, note, desktop = null, onCommands = () => {} }) {
  let list = [];
  let current = null;
  const resolved = new Map();

  // The window behind the page, and Windows' caption buttons over its top
  // strip, in the theme's colors (see app:theme in electron/main.js). A
  // theme only being looked at dresses the window too, but isn't kept for
  // its next start.
  const dress = ({ colors }) => desktop?.setTheme?.({ background: colors.bg, symbols: colors.accent, keep: !themeState().preview });
  dress(THEME);
  onTheme(dress);
  // The desktop app's Theme menus (electron/theme-menu.js): every theme and
  // the one in use, each with its palette command, which a click there runs
  // here, so it switches as choosing it in the palette does.
  const tellMenus = () => desktop?.setThemes?.({ current, themes: list.map((t) => ({ id: t.id, name: t.name, command: commandId(t.id) })) });

  // Uses theme `id`, here and in every window: the server keeps the choice
  // and tells the rest. Logged as one step (oplog.js) unless `log` is
  // false. Resolves to the theme once it's drawn here; a theme the server
  // doesn't have is an error, and says so.
  async function set(id, { log = true } = {}) {
    if (log) note("theme", id);
    const res = await post("/api/theme", { id });
    const body = await res.json().catch(() => ({}));
    if (!res.ok) {
      toast(`Couldn't change the theme: ${body.error ?? res.statusText}`, true);
      // A menu that checked it on the click is told what's still in use.
      tellMenus();
      throw new Error(body.error ?? `couldn't change the theme to ${id}`);
    }
    // The server tells every page, this one too, so the theme may be on its
    // way twice; either way it's drawn once, and this waits for it.
    await useTheme(body);
    await themeDrawn();
    sfx("success");
    current = body.id;
    return body;
  }

  // A theme whole, as the server resolves it (a file of yours may stand in
  // for a built-in), kept until the list changes.
  async function resolve(id) {
    if (!resolved.has(id)) {
      const res = await fetch(`/api/themes/${encodeURIComponent(id)}`);
      const body = await res.json().catch(() => ({}));
      if (!res.ok) throw new Error(body.error ?? `there's no theme called ${id}`);
      resolved.set(id, body);
    }
    return resolved.get(id);
  }

  // Every theme whole, in the list's order.
  const whole = () => Promise.all(list.map((t) => resolve(t.id).catch(() => null))).then((all) => all.filter(Boolean));

  // Each theme whole, and its fonts loaded, so moving to one in the
  // switcher or the gallery draws it at once.
  let warming = null;
  function warm() {
    warming ??= whole()
      .then((all) => Promise.all(all.map((t) => themeFonts(t))))
      .finally(() => (warming = null));
    return warming;
  }

  // Shows theme `id` on this page alone, without saving or logging it;
  // keep() makes it the one in use everywhere, revert() goes back. Looking
  // at the theme in use is going back to it.
  async function preview(id) {
    const theme = await resolve(id);
    await (id === current ? revertTheme() : previewTheme(theme));
    await themeDrawn();
    return theme;
  }
  // Keeping or going back ends a look through the switcher too.
  const keep = () => {
    switcher.hide();
    return themeState().preview ? set(themeState().preview) : Promise.resolve(null);
  };
  const revert = () => {
    switcher.hide();
    return revertTheme().then(themeDrawn);
  };

  // A copy of theme `from` (the one in use, unless it's another), as a file
  // of your own to edit (core/themefiles.js), which XenoNet then uses.
  // Resolves to { id, file }, or null if it couldn't.
  async function copy(from = current ?? THEME.id) {
    const taken = list.map((t) => t.id);
    let id = `${from}-mine`;
    for (let i = 2; taken.includes(id); i++) id = `${from}-mine-${i}`;
    const res = await post("/api/themes/copy", { from, id, name: `My ${list.find((t) => t.id === from)?.name ?? from}` });
    const body = await res.json().catch(() => ({}));
    if (!res.ok) {
      toast(`Couldn't copy the theme: ${body.error ?? res.statusText}`, true);
      return null;
    }
    toast(`Saved ${tildePath(body.file)}; edit it, and XenoNet follows`);
    await load();
    await set(id, { log: false }).catch(() => {});
    return { id, file: body.file };
  }

  // A theme of yours, in the file manager (Finder, Explorer).
  async function reveal(id) {
    const res = await post("/api/themes/reveal", { id });
    const body = await res.json().catch(() => ({}));
    if (!res.ok) toast(`Couldn't show the file: ${body.error ?? res.statusText}`, true);
    else if (!body.shown) toast(`The demo leaves your other apps alone; the file is ${tildePath(body.file)}`);
  }

  // The palette's commands: one for each theme, which logs itself as a
  // `theme` step; the switcher and the gallery, where only the theme you
  // keep is logged (as a `theme` step), so playing the log back doesn't
  // leave them open; and one to start your own (logged as a command).
  function commands() {
    return [
      { id: "theme-switcher", label: "Theme…", keywords: "themes look colors appearance switch preview try", logs: true, run: () => switcher.show() },
      { id: "theme-gallery", label: "Theme gallery", keywords: "themes look colors appearance browse preview swatches fonts", logs: true, run: () => gallery.show() },
      ...list.map((t) => ({ id: commandId(t.id), label: `Theme: ${t.name}${t.id === current ? " (in use)" : ""}`, keywords: `theme look colors appearance ${t.about}`, logs: true, run: () => set(t.id).catch(() => {}) })),
      { id: "theme-copy", label: "Make a theme of your own", keywords: "theme custom edit colors fonts sky grid music json", run: () => copy() },
    ];
  }

  // The themes there are and the one in use: at the start, after a switch
  // anywhere, and whenever a theme file changes. Calls that come together
  // (a change to the folder is news of both) share one fetch.
  let loading = null;
  function load() {
    loading ??= fetch("/api/themes")
      .then((r) => r.json())
      .then((res) => {
        list = res.themes;
        current = res.current;
        resolved.clear();
        onCommands(commands());
        tellMenus();
        gallery.refresh();
      })
      .catch(() => {})
      .finally(() => (loading = null));
    return loading;
  }

  const api = {
    load,
    set,
    preview,
    keep,
    revert,
    copy,
    reveal,
    whole,
    warm,
    get list() {
      return list;
    },
    get current() {
      return current;
    },
    // The theme drawn while you look at another, or null.
    get shown() {
      return themeState().preview;
    },
  };
  // The switcher and the gallery, made the first time either is opened.
  // Until then these stand in for them: closed, with nothing to show, and
  // anything that opens one loads both.
  let parts = null;
  let loadingParts = null;
  const loadParts = () =>
    (loadingParts ??= Promise.all([import("./theme-switcher.js"), import("./theme-gallery.js")]).then(([s, g]) => (parts = { switcher: s.createSwitcher({ themes: api, sfx }), gallery: g.createGallery({ themes: api, sfx, toast }) })));
  const switcher = {
    show: () => loadParts().then((p) => p.switcher.show()),
    hide: () => parts?.switcher.hide(),
    keep: () => loadParts().then((p) => p.switcher.keep()),
    cancel: () => (parts ? parts.switcher.cancel() : Promise.resolve()),
    step: (by) => loadParts().then((p) => p.switcher.step(by)),
    get isOpen() {
      return !!parts?.switcher.isOpen;
    },
    state: () => parts?.switcher.state() ?? null,
  };
  const gallery = {
    // Its cards are set in each theme's fonts, so they start loading too.
    show: (id) => loadParts().then((p) => (warm(), p.gallery.show(id))),
    hide: () => (parts ? parts.gallery.hide() : Promise.resolve()),
    keep: (id) => loadParts().then((p) => p.gallery.keep(id)),
    make: () => loadParts().then((p) => p.gallery.make()),
    refresh: () => parts?.gallery.refresh(),
    focus: (id) => loadParts().then((p) => (warm(), p.gallery.focus(id))),
    get isOpen() {
      return !!parts?.gallery.isOpen;
    },
    state: () => parts?.gallery.state() ?? null,
  };
  api.switcher = switcher;
  api.gallery = gallery;
  return api;
}
