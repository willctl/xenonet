// The only bridge between the page and the desktop: terminals, native
// notifications, recording films, the folder picker, commands from the
// tray and the macOS menu bar, the theme's colors for the window itself,
// the window key for the page's reports of you and for allowing actors,
// whether the window shows, and saving an actor's token to a file.
const { contextBridge, ipcRenderer } = require("electron");

contextBridge.exposeInMainWorld("xenonetDesktop", {
  platform: process.platform,
  updates: {
    act: (action, value) => ipcRenderer.invoke("app:updates", action, value),
    onState: (fn) => {
      const listener = (_e, state) => fn(state);
      ipcRenderer.on("app:update-state", listener);
      return () => ipcRenderer.removeListener("app:update-state", listener);
    },
  },
  terminal: {
    profiles: () => ipcRenderer.invoke("term:profiles"),
    list: () => ipcRenderer.invoke("term:list"),
    watch: (id, replay) => ipcRenderer.invoke("term:watch", id, replay),
    create: (opts) => ipcRenderer.invoke("term:create", opts),
    write: (id, data) => ipcRenderer.send("term:write", id, data),
    resize: (id, cols, rows) => ipcRenderer.send("term:resize", id, cols, rows),
    kill: (id) => ipcRenderer.invoke("term:kill", id),
    onData: (fn) => ipcRenderer.on("term:data", (_e, id, data, meta) => fn(id, data, meta)),
    onExit: (fn) => ipcRenderer.on("term:exit", (_e, id, code) => fn(id, code)),
    openExternal: (url) => ipcRenderer.invoke("app:open-external", url),
  },
  notify: (n) => ipcRenderer.send("app:notify", n),
  clearNotifications: (keys) => ipcRenderer.send("app:notify-clear", keys),
  hideWindow: () => ipcRenderer.send("app:hide"),
  // Films of the day: start recording the window, then stop (to an MP4 in
  // the videos folder).
  record: (opts) => ipcRenderer.invoke("app:record", opts),
  // The folder picker, for choosing the projects folder (src/root.js): the
  // folder picked, or null.
  chooseFolder: (opts) => ipcRenderer.invoke("app:choose-folder", opts),
  onSelect: (fn) => ipcRenderer.on("app:select", (_e, id) => fn(id)),
  onCommand: (fn) => ipcRenderer.on("app:command", (_e, cmd) => fn(cmd)),
  // The window's background behind the page and, on Windows, the caption
  // buttons drawn over its top strip: { background, symbols } as #rrggbb,
  // and keep: false for a theme the page is only looking at.
  setTheme: (look) => ipcRenderer.send("app:theme", look),
  // The themes and the one in use, for the Theme menus in the tray and the
  // macOS menu bar: { current, themes: [{ id, name, command }] }, where a
  // click there sends `command` back (onCommand), as the palette runs it.
  setThemes: (menu) => ipcRenderer.send("app:themes", menu),
  // The key for the routes only XenoNet's own window may use (src/you.js):
  // the desktop app gives it to this page alone, and null when it didn't
  // start the server. Kept in memory by the page, never in a URL.
  windowKey: () => ipcRenderer.invoke("app:window-key"),
  // Whether XenoNet's window shows, "shown" or "hidden" (minimized counts),
  // for the page's word on you (src/you.js): the page keeps drawing while
  // the window is hidden, so it never sees itself hidden. Asked once, then
  // told of each change. A test window that's never shown counts as shown.
  windowState: () => ipcRenderer.invoke("app:window-state"),
  onWindowState: (fn) => ipcRenderer.on("app:window-state", (_e, state) => fn(state)),
  // What the page's pace of drawing depends on that only the desktop app
  // knows (src/render-budget.js): { recording, driven, hold }, a recording
  // that needs every frame, a script or agent working the window, and the
  // screen locked or the computer asleep. Asked once, then told of each
  // change.
  drawing: () => ipcRenderer.invoke("app:drawing"),
  onDrawing: (fn) => ipcRenderer.on("app:drawing", (_e, state) => fn(state)),
  // An actor's token, shown once in the Actors panel, saved through a save
  // dialog to a file only you can read: { file, warning } (file with ~), or
  // null when you cancel. `slug` names the file it suggests.
  saveToken: (token, { slug } = {}) => ipcRenderer.invoke("app:save-token", { token, slug }),
  savePatch: (value) => ipcRenderer.invoke("app:save-patch", value),
});
