// Terminal drawer (desktop app only). xterm.js with its WebGL renderer in the
// page; real shells in the Electron main process via node-pty (ConPTY and
// PowerShell 7 on Windows, the login shell on macOS). Ctrl+` toggles it; new
// tabs open in the folder of whatever is selected in the graph.
import { Terminal } from "@xterm/xterm";
import { WebglAddon } from "@xterm/addon-webgl";
import { FitAddon } from "@xterm/addon-fit";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { SearchAddon } from "@xterm/addon-search";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { ImageAddon } from "@xterm/addon-image";
import "@xterm/xterm/css/xterm.css";
import { escapeHtml as h } from "./format.js";
import { isMac, keyLabel } from "./platform.js";
import { THEME, onTheme, withAlpha } from "./theme.js";
import { osc52Handler } from "./osc52.js";

const HEIGHT_KEY = "xenonet.terminal.height";

// `a` moved toward `b` by k (0 to 1), as #rrggbb.
const mix = (a, b, k) => `#${[1, 3, 5].map((i) => Math.round(parseInt(a.slice(i, i + 2), 16) * (1 - k) + parseInt(b.slice(i, i + 2), 16) * k).toString(16).padStart(2, "0")).join("")}`;

// The terminal's colors, from the theme in use: its surface and text, the
// cursor and selection in its accent, its grays for black and white, and
// its red and yellow. Green, blue, magenta and cyan keep their hues in
// every theme, since programs mean something by them (a diff's added lines,
// a branch name).
function palette({ colors: c } = THEME) {
  return {
    background: c.surfaceLow,
    foreground: c.text,
    cursor: c.accent,
    cursorAccent: c.surfaceLow,
    selectionBackground: withAlpha(c.accent, 0.28),
    selectionInactiveBackground: withAlpha(c.accent, 0.14),
    black: mix(c.surface, c.text, 0.025),
    red: c.danger,
    green: "#5dffc1",
    yellow: c.gold,
    blue: "#5fa8ff",
    magenta: "#d18cff",
    cyan: "#5fe1ff",
    white: mix(c.text, c.label, 0.2),
    brightBlack: c.dim,
    brightRed: mix(c.danger, "#ffffff", 0.25),
    brightGreen: "#9dffd9",
    brightYellow: mix(c.gold, "#ffffff", 0.3),
    brightBlue: "#9cc8ff",
    brightMagenta: "#e5b8ff",
    brightCyan: "#a6f1ff",
    brightWhite: "#ffffff",
  };
}

export function createTerminalDrawer({ api, sfx, toast, folderOf }) {
  const root = document.getElementById("terminal");
  root.innerHTML = `
    <div class="tm-grip" title="Drag to resize"></div>
    <header class="tm-bar">
      <span class="tm-k">›_ terminal</span>
      <ol class="tm-tabs"></ol>
      <div class="tm-new">
        <button class="tm-btn" data-tm-new title="New tab · ${keyLabel(isMac ? "Mod+T" : "Ctrl+Shift+T")}">+</button>
        <button class="tm-btn" data-tm-profiles title="Choose a shell">▾</button>
        <ol class="tm-menu" hidden></ol>
      </div>
      <span class="tm-grow"></span>
      <input class="tm-find" placeholder="Find · ${keyLabel(isMac ? "Mod+F" : "Ctrl+Shift+F")}" hidden spellcheck="false" />
      <button class="tm-btn" data-tm-find title="Find">⌕</button>
      <button class="tm-btn" data-tm-max title="Maximize">⤢</button>
      <button class="tm-btn" data-tm-hide title="Hide · ${keyLabel("Ctrl+`")}">✕</button>
    </header>
    <div class="tm-body"></div>`;
  const tabsEl = root.querySelector(".tm-tabs");
  const body = root.querySelector(".tm-body");
  const menu = root.querySelector(".tm-menu");
  const find = root.querySelector(".tm-find");
  const tabs = [];
  let active = null;
  let profiles = [];
  let open = false;

  try {
    const saved = Number(localStorage.getItem(HEIGHT_KEY));
    if (saved > 120) document.documentElement.style.setProperty("--tm-height", `${saved}px`);
  } catch {}

  // Every tab takes a new theme's colors where it is.
  onTheme(() => {
    for (const t of tabs) t.term.options.theme = palette();
  });

  api.onData((id, data, meta) => {
    const t = tabs.find((t) => t.id === id);
    if (!t) return;
    // Replayed terminal queries already got their answers. Sending them to
    // the shell again would turn device responses into command input.
    if (meta?.replay) t.replaying = true;
    t.term.write(data, () => { if (meta?.replay) t.replaying = false; });
  });
  api.onExit((id, code) => {
    const t = tabs.find((x) => x.id === id);
    if (!t) return;
    t.exited = true;
    const muted = [1, 3, 5].map((i) => parseInt(THEME.colors.muted.slice(i, i + 2), 16)).join(";");
    t.term.write(`\r\n\x1b[38;2;${muted}m[the shell exited with code ${code}; press any key to close the tab]\x1b[0m`);
    renderTabs();
  });

  function renderTabs() {
    tabsEl.innerHTML = tabs
      .map(
        (t) => `<li class="tm-tab ${t === active ? "on" : ""} ${t.exited ? "dead" : ""}" data-tm-tab="${t.id}" title="${h(t.cwd ?? "")}">
          <span class="tm-dot"></span><span class="tm-title">${h(t.title)}</span><button class="tm-x" data-tm-close="${t.id}">×</button>
        </li>`,
      )
      .join("");
  }

  function activate(t) {
    active = t;
    for (const x of tabs) x.el.hidden = x !== t;
    renderTabs();
    if (t) {
      requestAnimationFrame(() => {
        fitTab(t);
        t.term.focus();
      });
    }
  }

  function fitTab(t) {
    if (!t || t.el.hidden || !open) return;
    try {
      t.fit.fit();
      api.resize(t.id, t.term.cols, t.term.rows);
    } catch {}
  }

  async function newTab({ profile, cwd, restored } = {}) {
    if (!profiles.length) profiles = await api.profiles();
    const prof = profiles.find((p) => p.id === profile) ?? profiles[0];
    const el = document.createElement("div");
    el.className = "tm-pane";
    body.appendChild(el);
    const term = new Terminal({
      ...(restored ? { cols: restored.cols, rows: restored.rows } : {}),
      fontFamily: isMac ? '"SF Mono", Menlo, "JetBrains Mono", monospace' : '"Cascadia Code", "Cascadia Mono", "JetBrains Mono", Consolas, monospace',
      fontSize: 13,
      lineHeight: 1.15,
      letterSpacing: 0,
      cursorBlink: true,
      cursorStyle: "bar",
      cursorInactiveStyle: "outline",
      scrollback: 20000,
      allowProposedApi: true,
      // Option types characters (@, €, ñ on many layouts), as in Terminal and
      // VS Code; Option+arrows still move by word.
      macOptionIsMeta: false,
      macOptionClickForcesSelection: true,
      theme: palette(),
      minimumContrastRatio: 1,
      smoothScrollDuration: 80,
      rescaleOverlappingGlyphs: true,
    });
    const fit = new FitAddon();
    const search = new SearchAddon();
    term.loadAddon(fit);
    term.loadAddon(search);
    term.loadAddon(new WebLinksAddon((_e, uri) => api.openExternal(uri)));
    const unicode = new Unicode11Addon();
    term.loadAddon(unicode);
    term.unicode.activeVersion = "11";
    // Programs may set the clipboard (OSC 52), and are never told what's on
    // it (see osc52.js).
    term.parser.registerOscHandler(52, osc52Handler((text) => { if (!tabs.find((t) => t.term === term)?.replaying) navigator.clipboard.writeText(text).catch(() => {}); }));
    term.loadAddon(new ImageAddon());
    term.open(el);
    // GPU rendering; fall back to the DOM renderer if the context is lost.
    let webgl = null;
    try {
      webgl = new WebglAddon();
      webgl.onContextLoss(() => webgl?.dispose());
      term.loadAddon(webgl);
    } catch {
      webgl = null;
    }
    fit.fit();
    const created = restored ?? await api.create({ profile: prof.id, cwd, cols: term.cols, rows: term.rows });
    const t = { id: created.id, term, fit, search, el, title: prof.label, cwd: created.cwd, profile: prof.id, renderer: webgl ? "webgl" : "dom", exited: false };
    tabs.push(t);
    await api.watch?.(t.id, !!restored);

    term.onData((d) => {
      if (t.replaying) return;
      if (t.exited) return closeTab(t);
      api.write(t.id, d);
    });
    term.onTitleChange((title) => {
      t.title = title.replace(/^.*[\\/]/, "").slice(0, 32) || prof.label;
      renderTabs();
    });
    term.attachCustomKeyEventHandler((e) => {
      if (e.type !== "keydown") return true;
      const k = e.key.toLowerCase();
      if (isMac) return macKey(e, k, t);
      // Windows Terminal conventions: Ctrl+C copies when there's a selection,
      // Ctrl+Shift+C/V copy and paste, Ctrl+V pastes text.
      if (e.ctrlKey && k === "c" && (e.shiftKey || term.hasSelection())) {
        if (term.hasSelection()) navigator.clipboard.writeText(term.getSelection());
        term.clearSelection();
        return false;
      }
      if (e.ctrlKey && k === "v") {
        navigator.clipboard.readText().then((text) => text && term.paste(text));
        return false;
      }
      if (e.ctrlKey && e.shiftKey && k === "f") {
        showFind();
        return false;
      }
      if (e.ctrlKey && e.shiftKey && k === "t") {
        newTab({ profile: t.profile, cwd: t.cwd });
        return false;
      }
      if (e.ctrlKey && e.shiftKey && k === "w") {
        closeTab(t);
        return false;
      }
      if (e.ctrlKey && e.key === "`") return false;
      return true;
    });
    el.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      if (term.hasSelection()) {
        navigator.clipboard.writeText(term.getSelection());
        term.clearSelection();
        toast("Copied");
      } else navigator.clipboard.readText().then((text) => text && term.paste(text));
    });
    activate(t);
    return t;
  }

  // Terminal and iTerm conventions. Control keys all reach the shell (Ctrl+C
  // always interrupts). ⌘C, ⌘V and ⌘W go to the menu bar, whose Edit menu
  // copies and pastes and whose Close closes the focused tab.
  const SHELL_KEYS = { arrowleft: "\x01", arrowright: "\x05", backspace: "\x15" }; // line start, line end, delete line
  function macKey(e, k, t) {
    if (!e.metaKey || e.ctrlKey) return !(e.ctrlKey && e.key === "`");
    const claim = (fn) => (e.preventDefault(), fn(), false);
    if (k === "t" && !e.shiftKey) return claim(() => newTab({ profile: t.profile, cwd: t.cwd }));
    if (k === "f") return claim(showFind);
    if (k === "a") return claim(() => t.term.selectAll());
    if (SHELL_KEYS[k] && !e.shiftKey && !e.altKey) return claim(() => !t.exited && api.write(t.id, SHELL_KEYS[k]));
    return false;
  }

  // For ⌘W: closes the tab whose terminal has focus. False if none does.
  function closeFocusedTab() {
    const t = tabs.find((x) => x.el.contains(document.activeElement));
    if (!t || !open) return false;
    closeTab(t);
    return true;
  }

  function closeTab(t) {
    const i = tabs.indexOf(t);
    if (i < 0) return;
    api.kill(t.id);
    t.term.dispose();
    t.el.remove();
    tabs.splice(i, 1);
    if (active === t) activate(tabs[Math.max(0, i - 1)] ?? null);
    else renderTabs();
    if (!tabs.length) hide();
  }

  function showFind() {
    find.hidden = false;
    find.focus();
    find.select();
  }

  function show({ cwd, profile, fresh = false } = {}) {
    const wasOpen = open;
    open = true;
    root.hidden = false;
    document.body.classList.add("terminal-open");
    requestAnimationFrame(() => root.classList.add("open"));
    if (!wasOpen) sfx("terminal");
    if (!tabs.length || fresh) return newTab({ cwd, profile });
    activate(active ?? tabs[0]);
  }

  function hide() {
    if (!open) return;
    open = false;
    root.classList.remove("open");
    document.body.classList.remove("terminal-open");
    setTimeout(() => !open && (root.hidden = true), 220);
    sfx("close");
  }

  function toggle() {
    open ? hide() : show({ cwd: folderOf() });
  }

  root.addEventListener("click", async (e) => {
    const tab = e.target.closest("[data-tm-tab]");
    const closeBtn = e.target.closest("[data-tm-close]");
    if (closeBtn) return closeTab(tabs.find((t) => t.id === Number(closeBtn.dataset.tmClose)));
    if (tab) return activate(tabs.find((t) => t.id === Number(tab.dataset.tmTab)));
    if (e.target.closest("[data-tm-new]")) return newTab({ profile: active?.profile, cwd: folderOf() ?? active?.cwd });
    if (e.target.closest("[data-tm-profiles]")) {
      if (!profiles.length) profiles = await api.profiles();
      menu.innerHTML = profiles.map((p) => `<li data-tm-profile="${h(p.id)}">${h(p.label)}<small>${h(p.file)}</small></li>`).join("");
      menu.hidden = !menu.hidden;
      return;
    }
    const prof = e.target.closest("[data-tm-profile]");
    if (prof) {
      menu.hidden = true;
      return newTab({ profile: prof.dataset.tmProfile, cwd: folderOf() ?? active?.cwd });
    }
    if (e.target.closest("[data-tm-find]")) return find.hidden ? showFind() : (find.hidden = true);
    if (e.target.closest("[data-tm-max]")) {
      root.classList.toggle("max");
      sfx("toggle");
      return setTimeout(() => fitTab(active), 240);
    }
    if (e.target.closest("[data-tm-hide]")) return hide();
  });

  find.addEventListener("keydown", (e) => {
    const { accent, gold } = THEME.colors;
    if (e.key === "Enter") active?.search.findNext(find.value, { decorations: { matchOverviewRuler: accent, activeMatchColorOverviewRuler: gold, matchBackground: `${accent}44`, activeMatchBackground: `${gold}66` } });
    if (e.key === "Escape") {
      find.hidden = true;
      active?.search.clearDecorations();
      active?.term.focus();
    }
  });

  // Drag the top edge to resize; remembered across reloads.
  root.querySelector(".tm-grip").addEventListener("pointerdown", (e) => {
    const grip = e.currentTarget;
    grip.setPointerCapture(e.pointerId);
    const move = (ev) => {
      const hgt = Math.max(160, Math.min(innerHeight - 120, innerHeight - ev.clientY));
      document.documentElement.style.setProperty("--tm-height", `${hgt}px`);
      fitTab(active);
    };
    grip.onpointermove = move;
    grip.onpointerup = () => {
      grip.onpointermove = null;
      try {
        localStorage.setItem(HEIGHT_KEY, String(parseInt(getComputedStyle(root).getPropertyValue("--tm-height"))));
      } catch {}
    };
  });

  new ResizeObserver(() => fitTab(active)).observe(body);

  addEventListener(
    "keydown",
    (e) => {
      if (e.ctrlKey && (e.key === "`" || e.code === "Backquote")) {
        e.preventDefault();
        e.stopPropagation();
        toggle();
      }
    },
    true,
  );

  api.list?.().then(async (list) => {
    for (const restored of list || []) await newTab({ profile: restored.profile, cwd: restored.cwd, restored });
  }).catch((err) => toast(`Could not restore terminals: ${err.message}`));

  return {
    show,
    hide,
    toggle,
    newTab,
    write(id, text) {
      const t = tabs.find((t) => t.id === id) ?? (id == null ? active : null);
      if (!t || t.exited) throw new Error("That terminal is not running.");
      if (typeof text !== "string") throw new Error("Terminal input must be text.");
      api.write(t.id, text);
    },
    close(id) { const t = tabs.find((t) => t.id === id); if (t) closeTab(t); },
    closeFocusedTab,
    get isOpen() {
      return open;
    },
    get debug() {
      return tabs.map((t) => ({ id: t.id, title: t.title, cwd: t.cwd, renderer: t.renderer, cols: t.term.cols, rows: t.term.rows, exited: t.exited }));
    },
    bufferText(n = 40) {
      const b = active?.term.buffer.active;
      if (!b) return "";
      const out = [];
      for (let i = Math.max(0, b.length - n); i < b.length; i++) out.push(b.getLine(i)?.translateToString(true) ?? "");
      return out.join("\n").trimEnd();
    },
  };
}
