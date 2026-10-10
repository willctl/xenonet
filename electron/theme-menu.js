// The Theme menu in the tray and the macOS menu bar (tray.js, mac.js):
// every theme, the one in use checked, each running the page's own command
// for it, as choosing it in the palette does, so the theme switches live
// and the step is logged like any other. The page says which themes there
// are and which is in use whenever that changes (src/themes-ui.js, through
// the preload's setThemes), and this checks what it says. No Electron
// here, so it can be tested on its own.

const ID = /^[a-z0-9][a-z0-9-]{0,40}$/;
const COMMAND = /^theme-(use-)?[a-z0-9][a-z0-9-]{0,40}$/;
const THEMES_MAX = 64;
const NAME_MAX = 60;

// The page's word, as the menus take it: { current, themes: [{ id, name,
// command }] }, or null when there's nothing to list. A theme's name is a
// theme file's own words, so it's kept to one short line; ids and commands
// are only ever a theme's.
export function themeMenuOf(said) {
  if (!said || typeof said !== "object" || !Array.isArray(said.themes)) return null;
  const themes = [];
  for (const t of said.themes.slice(0, THEMES_MAX)) {
    if (!t || typeof t !== "object" || typeof t.id !== "string" || !ID.test(t.id) || typeof t.command !== "string" || !COMMAND.test(t.command)) continue;
    if (themes.some((x) => x.id === t.id)) continue;
    const name = (typeof t.name === "string" ? t.name : "").replace(/\s+/g, " ").trim().slice(0, NAME_MAX) || t.id;
    themes.push({ id: t.id, name, command: t.command });
  }
  if (!themes.length) return null;
  return { current: themes.some((t) => t.id === said.current) ? said.current : null, themes };
}

// The Theme submenu: a radio item for each theme, the one in use checked;
// `pick(theme)` runs when one is chosen. Windows and Linux read & in a
// label as the key that picks it (`mnemonics`), so a name's own & is
// doubled there to show as one.
export function themeItems(menu, pick, { mnemonics = false } = {}) {
  return (menu?.themes ?? []).map((t) => ({ label: mnemonics ? t.name.replaceAll("&", "&&") : t.name, type: "radio", checked: t.id === menu.current, click: () => pick(t) }));
}
