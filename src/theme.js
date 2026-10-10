// The console's colors, fonts, sky, glow and music, from the theme in use
// (core/themes.js). One theme is in use at a time, and applyTheme changes
// it in place, without a reload: the stylesheet's tokens change at once,
// and everything drawn with the theme draws itself again (onTheme). The
// theme applies before anything is drawn, from the copy this browser keeps;
// once the page is up it follows the server's (syncTheme): another window,
// xenonet theme, or an edit to a theme of your own.
//
// Read THEME and COLORS when drawing, not once at import: they change.
import { themes, complete, ringColors, DEFAULT_THEME } from "../core/themes.js";

const STORE = "xenonet.theme";

// A built-in theme is kept by id, so it follows the code; your own, whole.
function kept() {
  try {
    const k = JSON.parse(localStorage.getItem(STORE));
    if (k?.builtIn) return themes()[k.id] ?? null;
    return k?.theme?.colors ? k.theme : null;
  } catch {
    return null;
  }
}

function keep(theme) {
  try {
    localStorage.setItem(STORE, JSON.stringify(theme.builtIn ? { id: theme.id, builtIn: true } : { theme }));
  } catch {}
}

export let THEME = complete(kept() ?? themes()[DEFAULT_THEME]);

// By role, kept up to date in place, so COLORS.claude is always the theme
// in use's: the colors, the tool families, the wiki's page types and the
// identity rings.
const ROLES = Object.keys(themes()[DEFAULT_THEME].colors);
export const COLORS = {};
export const CATEGORY_COLORS = {};
export const PAGE_COLORS = {};
export const RING_COLORS = [];
function setColors(theme) {
  for (const k of ROLES) COLORS[k] = theme.colors[k];
  Object.assign(CATEGORY_COLORS, theme.categories);
  Object.assign(PAGE_COLORS, theme.pages);
  RING_COLORS.splice(0, RING_COLORS.length, ...ringColors(theme));
}
setColors(THEME);

// The panels' body text: the theme's body font, or its display font when it
// names none (contracts, section 4).
const bodyFont = (theme) => theme.fonts.body ?? theme.fonts.display;

// The stylesheet's tokens (see :root in style.css): each color by role and
// its -rgb twin, the tool families (--cat-*), the wiki's page types
// (--page-*), the identity rings (--ring-*), the fonts, and the stats
// panel's chart colors. A panel that sets its colors with var() follows a
// change of theme by itself.
const CSS_NAMES = { surfaceLow: "surface-low", native: "violet" };
const CHARTS = { m1: "m1", m2: "m2", m3: "m3", m4: "m4", m5: "m5", other: "other", one: "one", bad: "bad", cacheRead: "k-cacheRead", input: "k-input", cacheWrite: "k-cacheWrite", output: "k-output" };
const rgb = (hex) => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16)).join(" ");
function applyCss(theme) {
  const root = document.documentElement.style;
  const color = (name, hex) => {
    root.setProperty(`--${name}`, hex);
    root.setProperty(`--${name}-rgb`, rgb(hex));
  };
  for (const k of ROLES) color(CSS_NAMES[k] ?? k, theme.colors[k]);
  for (const [k, v] of Object.entries(theme.categories)) root.setProperty(`--cat-${k}`, v);
  for (const [k, v] of Object.entries(theme.pages)) root.setProperty(`--page-${k}`, v);
  RING_COLORS.forEach((c, i) => color(`ring-${i}`, c));
  root.setProperty("--display", theme.fonts.display);
  root.setProperty("--body", bodyFont(theme));
  root.setProperty("--mono", theme.fonts.mono);
  for (const [key, name] of Object.entries(CHARTS)) if (theme.charts?.[key]) root.setProperty(`--st-${name}`, theme.charts[key]);
  document.documentElement.dataset.theme = theme.id;
}
if (typeof document !== "undefined") applyCss(THEME);

// A role's color as CSS that follows the theme, such as var(--claude), and
// a tool family's. A family is one XenoNet knows, never a word from the
// data (a host's tool counts, say): this goes into inline styles.
export const colorVar = (role) => `var(--${CSS_NAMES[role] ?? role})`;
export const categoryVar = (category) => `var(--cat-${Object.hasOwn(CATEGORY_LABEL, category) ? category : "other"})`;

// A color (#rrggbb) with some transparency, for drawing on a canvas.
export const withAlpha = (hex, alpha) => `rgba(${rgb(hex).replaceAll(" ", ",")},${alpha})`;

// The fonts a theme draws with, loaded before it's drawn: the map's labels
// and the poster are drawn onto canvases, which don't draw again when a
// font arrives late. A font that doesn't come within a few seconds is
// drawn in the next one along.
export function themeFonts(theme = THEME) {
  const { display, mono } = theme.fonts;
  const body = bodyFont(theme);
  const faces = [`600 32px ${display}`, `500 32px ${display}`, `400 14px ${mono}`, `500 14px ${mono}`, ...(body !== display ? [`400 14px ${body}`, `600 14px ${body}`] : [])];
  const loaded = Promise.all(faces.map((f) => document.fonts.load(f).catch(() => [])));
  return Promise.race([loaded, new Promise((r) => setTimeout(r, 3000))]).then(() => {});
}

// ---------- changing it ----------

const listeners = new Set();
let turn = 0; // the latest change asked for; an earlier one still loading its fonts gives way
let drawn = 0; // the change drawn last
let applied = 0; // changes drawn so far
let inUse = THEME; // what this page keeps: the server's choice
let previewing = null; // the id shown instead, while you look at another
const waiting = []; // themeDrawn's callers
const settle = () => drawn === turn && waiting.splice(0).forEach((resolve) => resolve());

// Resolves once no change of theme is still on its way: the latest one
// asked for is drawn.
export const themeDrawn = () => (drawn === turn ? Promise.resolve() : new Promise((resolve) => waiting.push(resolve)));

// Calls fn(theme, previous) after every change, once the stylesheet and
// COLORS are the new theme's. Returns a function that stops it.
export function onTheme(fn) {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

// Draws everything in `theme` (a theme from the server, or one from
// core/themes.js), once its fonts are in. `keep` saves it as this
// browser's. Resolves true once it's drawn, or false if a later change
// came first.
export async function applyTheme(theme, { keep: save = true } = {}) {
  const next = complete(theme);
  const mine = ++turn;
  await themeFonts(next);
  if (mine !== turn) return false;
  const previous = THEME;
  THEME = next;
  setColors(next);
  applyCss(next);
  if (save) keep(next);
  drawn = mine;
  // One part failing to draw mustn't stop the rest; it's reported as the
  // page's error.
  for (const fn of [...listeners]) {
    try {
      fn(next, previous);
    } catch (err) {
      if (globalThis.reportError) reportError(err);
      else console.error(err);
    }
  }
  applied++;
  settle();
  return true;
}

const same = (a, b) => a === b || JSON.stringify(a) === JSON.stringify(b);

// Draws `theme` unless it's the one drawn already; then a change still
// loading its fonts gives way to it all the same, so the last word wins.
function show(theme, { keep: save = true } = {}) {
  if (!same(theme, THEME)) return applyTheme(theme, { keep: save });
  drawn = ++turn;
  if (save) keep(theme);
  settle();
  return Promise.resolve(false);
}

// The server's theme: once the page is up, and whenever the server says it
// changed. While you're looking at another (previewTheme), it's what you
// go back to. Resolves true once drawn, false if there was nothing to draw.
export function syncTheme(theme) {
  if (!theme?.colors) return Promise.resolve(false);
  inUse = complete(theme);
  return previewing ? Promise.resolve(false) : show(inUse);
}

// A theme chosen on this page, as the server has it now: the end of any
// preview.
export function useTheme(theme) {
  previewing = null;
  return syncTheme(theme);
}

// Shows `theme` on this page alone, to look at: nothing is saved, and
// revertTheme goes back to the theme in use.
export function previewTheme(theme) {
  previewing = theme.id;
  return show(complete(theme), { keep: false });
}

export function revertTheme() {
  previewing = null;
  return show(inUse);
}

// What's drawn, what's in use, and how many changes have been drawn, for
// scripts and tests (driver.js).
export const themeState = () => ({ id: THEME.id, name: THEME.name, builtIn: !!THEME.builtIn, inUse: inUse.id, preview: previewing, applied, pending: drawn !== turn });

// ---------- by kind ----------

// What an agent is doing, by tool family.
export const CATEGORY_LABEL = { shell: "shell", edit: "edit", read: "read", web: "web", mcp: "mcp", agent: "agent", code: "code", other: "other" };

export const APP_LABEL = { claude: "Claude Code", codex: "Codex" };
export const APP_DESKTOP = { claude: "Claude desktop", codex: "ChatGPT desktop" };

// Which color a node is drawn in: a role in COLORS, or a page type.
function roleOf(node) {
  switch (node.kind) {
    case "core":
    case "host":
      return "core";
    case "project":
      return node.satelliteOf ? "satellite" : "project";
    case "native":
    case "resource":
    case "wiki":
      return node.kind;
    case "zone":
      return node.app && COLORS[node.app] ? node.app : "zone";
    default:
      return COLORS[node.app] ? node.app : "zone";
  }
}

export function nodeColor(node) {
  if (node.kind === "page") return PAGE_COLORS[node.type] ?? PAGE_COLORS.topic;
  return COLORS[roleOf(node)];
}

// The same, as CSS that follows the theme.
export function nodeColorVar(node) {
  if (node.kind === "page") return `var(--page-${PAGE_COLORS[node.type] ? node.type : "topic"})`;
  return colorVar(roleOf(node));
}
