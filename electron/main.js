// XenoNet desktop app. Runs the XenoNet server in an Electron utility
// process (or reuses one already listening), shows the UI in a frameless
// window, hosts real shells for the terminal drawer, and lives in the tray
// (the menu bar on macOS).
import { app, BrowserWindow, ipcMain, Notification, powerMonitor, screen, session, shell, nativeImage } from "electron";
import { execFile } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { registerServicePty } from "./service-pty.js";
import { connectService, ensureService, runtimeDir } from "../core/components/runtime.js";
import { currentComponent } from "../core/components/packages.js";
import { installBundle, installedRuntime, recoverServiceUpdate } from "../core/components/install.js";
import { componentUpdates, updateComponent } from "../core/components/updates.js";
import { desktopClient } from "./service-client.js";
import { desktopServer, pageHeaders } from "./desktop-server.js";
import { createUpdates } from "./updates.js";
import { registerWindowsIdentity, windowsToastActivator } from "./windows-identity.js";
import { createTray } from "./tray.js";
import { launchedAtLogin } from "./login.js";
import { loadShellEnv } from "./shellenv.js";
import { prefs } from "./prefs.js";
import { rustBinary, missingBinary } from "./service-runtime.js";
import { installMenu, offerMove, askAboutLogin, uninstall } from "./mac.js";
import { themeMenuOf } from "./theme-menu.js";
import { instanceFrom } from "../core/instances.js";
import { createControl } from "./control.js";
import { sameOrigin, fromOwnPage, devtoolsPort } from "./trust.js";
import { createRecorder } from "./recorder.js";
import { FILM_FRAMES, DEFAULT_FRAME, fitFrame } from "../core/frames.js";
import { appDir } from "../core/dirs.js";
import { createLog, logDir } from "../core/log.js";
import { isToken } from "../core/actors/tokens.js";
import { writeTokenFile, gitTreeOf } from "../core/actors/token-file.js";
import { tilde } from "../core/paths.js";
import { patchExport } from "./patch-export.js";
import { iconImage, iconDataUrl, sheetDataUrl, artDataUrl, dmgArtDataUrl, pkgArtDataUrl, bmpFromImage, packIco, packIcns, pngWithDpi, closePainter } from "./icons.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
// A copy running from source (`npm run app`, or XenoNet Dev.app from
// `npm run dev:install`) is XenoNet Dev, with its own settings, port, layout
// and search index, so it runs beside an installed XenoNet instead of handing
// off to it or writing over its files. It runs as one of a few instances
// (see instances.js): your sessions, the demo, or the tests' copy of the
// demo, each its own app, so they run side by side too.
const fromSource = !app.isPackaged;
const packagedTest = app.isPackaged && process.env.XENONET_DEMO === "1" && process.env.XENONET_INSTANCE === "test";
if (process.platform === "linux" && process.env.XENONET_SOFTWARE_RENDERING === "1" && (fromSource || packagedTest)) {
  app.commandLine.appendSwitch("use-angle", "swiftshader");
  app.commandLine.appendSwitch("enable-unsafe-swiftshader");
  app.commandLine.appendSwitch("ignore-gpu-blocklist");
}
const instance = fromSource || packagedTest ? instanceFrom() : null;
if (fromSource || packagedTest) {
  app.setName(instance.name);
  app.setPath("userData", instance.id === "test" && process.env.XENONET_TEST_PROFILE ? process.env.XENONET_TEST_PROFILE : path.join(app.getPath("appData"), instance.name));
}
// XenoNet Dev.app is a renamed Electron whose only code is a pointer here.
const devBundle = fromSource && path.resolve(app.getAppPath()) !== root;
// Demo mode (XENONET_DEMO=1) gets its own port so it never reuses a real server.
const PORT = fromSource ? instance.port : Number(process.env.XENONET_PORT ?? (process.env.XENONET_DEMO === "1" ? 47841 : 47840));
const SERVICE_BASE = `http://127.0.0.1:${PORT}`;
// The service is the Rust binary, run standalone as an installed one is
// (service-runtime.js), from a checkout as well.
let BASE = SERVICE_BASE;
let serviceClient = null;
let frontendServer = null;
// Unpackaged runs serve the page through Vite (hot reload) unless --prod is
// passed.
const dev = fromSource && !process.argv.includes("--prod");
// How the window first appears (XENONET_SHOW): "front", as usual; "inactive",
// shown without taking focus from the app you're in, which is how scripts and
// agents start XenoNet Dev; or "hidden", never shown, for tests. A hidden window
// still renders, can be screenshotted and takes input; on Windows it sits off
// the screen instead. Hidden, XenoNet also stays silent: no sound, no
// notifications, no tray icon. XENONET_OFFSCREEN=1 is the old name for hidden.
const SHOW = process.env.XENONET_OFFSCREEN === "1" ? "hidden" : (process.env.XENONET_SHOW ?? "front");
let offscreen = SHOW === "hidden";
// A XenoNet Dev window the dev tools start (core/control.js: dev, demo and
// test) makes no sound, hidden or shown, so agents driving it stay quiet.
const MUTED = process.env.XENONET_MUTED === "1";
const isMac = process.platform === "darwin";
let startHidden = process.argv.includes("--hidden");

// Remote debugging for automated visual checks: XenoNet Dev only, opt-in
// (see trust.js).
const cdpPort = devtoolsPort({ fromSource });
if (cdpPort) app.commandLine.appendSwitch("remote-debugging-port", cdpPort);
if (fromSource && instance.id === "test" && process.env.XENONET_SOFTWARE_RENDERING === "1") {
  // Hosted runners have no usable GPU. SwiftShader is only enabled for
  // the isolated test world, never for an installed app or real sessions.
  app.commandLine.appendSwitch("use-gl", "angle");
  app.commandLine.appendSwitch("use-angle", "swiftshader");
  app.commandLine.appendSwitch("enable-unsafe-swiftshader");
} else app.commandLine.appendSwitch("enable-gpu-rasterization");
// Source runs keep their notification identity separate from installed XenoNet.
const APP_ID = app.isPackaged ? "dev.xenonet.app" : "dev.xenonet.app.dev";
app.setAppUserModelId(APP_ID);
const toastActivator = app.isPackaged ? "{81884AE8-79A6-4872-94B9-FB16A2E97AC9}" : "{C7A8A779-18B8-4DF9-BAF8-7BF6C86845EC}";
if (process.platform === "win32") app.setToastActivatorCLSID(toastActivator);

// One-shot tool modes that do their job and exit:
//   --make-icons <dir>     icon.png (1024), icon.ico (16-256), a preview sheet and
//                          the installer's sidebar and header bitmaps
//   --make-icon <file.png> one render; `--badge <working>,<needs>` for tray states
//   --install-shortcut     a Desktop shortcut carrying the app's ID and icon (Windows)
// and on macOS, where there's no separate uninstaller:
//   --uninstall [--purge]  quit, move XenoNet to the Trash, stop opening at login,
//                          and with --purge remove its data; a running XenoNet
//                          is asked to do it
const iconArg = process.argv.indexOf("--make-icon");
const iconsArg = process.argv.indexOf("--make-icons");
const shortcutArg = process.argv.indexOf("--install-shortcut");
const toolMode = iconArg >= 0 || iconsArg >= 0 || shortcutArg >= 0;
const uninstallMode = isMac && process.argv.includes("--uninstall");

// A second launch hands its arguments to the running XenoNet and quits.
const gotLock = toolMode || app.requestSingleInstanceLock();
// An --uninstall handed to the running XenoNet leaves at once: its own bundle is
// about to move to the Trash.
if (!gotLock) uninstallMode ? app.exit(0) : app.quit();

let win = null;
// Desktop's error log (core/log.js), beside the service's in the state
// folder: the page's process ending or hanging, a load that fails, and
// anything thrown here that nothing caught. The demo instances keep none.
const desktopLog = createLog({ dir: instance?.demo ? null : logDir(fromSource ? appDir("state", { name: instance.name }) : appDir("state")), component: "desktop", instance: instance?.id ?? "installed", version: app.getVersion() });
process.on("uncaughtExceptionMonitor", (err) => desktopLog.error("desktop.uncaught", err?.message ?? String(err), { err }));
let pty = null;
let quitting = false;
// How long a loaded page has to start before it's loaded again.
const BOOT_MS = 30e3;
let updates = null;
// The window key (core/actors/key.js), from the service this app connects
// to, for the routes only XenoNet's own window may use. Held here while
// connected, and given only to XenoNet's own page (app:window-key); never
// written down.
let windowKey = null;
// What quitting stops: Desktop's hold on the terminal's shells (the
// service keeps them), and its connection to the service, which hears
// first that the window closed (you went offline). An installed XenoNet's
// service keeps running; a service XenoNet Dev started goes with it, as the
// server it used to run in-process did, so each start of a dev, demo or
// test instance gets a fresh one.
function stop() {
  quitting = true;
  updates?.dispose();
  pty?.dispose();
  tellWindow("closed");
  serviceClient?.close();
  frontendServer?.close();
  if (ownService) try { process.kill(ownService); } catch {}
}

// Whether the window shows, for the server's word on you and the page's
// (core/actors/you.js, src/you.js): hidden or minimized is away. A hidden
// test window (XENONET_SHOW=hidden) stands in for a visible one, as it does
// for screenshots and input.
function windowState() {
  if (!win || win.isDestroyed()) return null;
  return offscreen || (win.isVisible() && !win.isMinimized()) ? "shown" : "hidden";
}
function tellWindow(state = windowState()) {
  if (!state) return;
  // The page hears it too (src/you.js, and src/render-budget.js, which
  // draws nothing while it's hidden). It goes with a window that closes.
  if (state !== "closed") win?.webContents.send("app:window-state", state);
  serviceClient?.request("window", state).catch(() => {});
}
// XenoNet Dev's control channel, for scripts, tests and agents (control.js).
let control = null;

// ---------- how often the page draws (src/render-budget.js) ----------
//
// The page keeps its own pace, and draws nothing while the window can't be
// seen. What only this process knows, it tells the page: a recording that
// needs every frame (recorder.js), a script, test or agent working the
// window through the control channel, which needs it drawn even while
// hidden, and the screen locked or the computer going to sleep. Chromium
// itself pauses a page that's hidden, minimized or, on macOS, covered by
// other windows, unless background throttling is off: it's off for a test
// window that's never shown (which stands in for a visible one), and while
// recording or driven, so those get every frame wherever the window is.
let driven = false;
const held = { locked: false, asleep: false };
let throttling = null;
const drawingState = () => ({ recording: !!recorder?.recording, driven, hold: held.locked || held.asleep });
function tellDrawing() {
  if (!win || win.isDestroyed()) return;
  const allowed = !offscreen && !recorder?.recording && !driven;
  if (allowed !== throttling) {
    throttling = allowed;
    win.webContents.setBackgroundThrottling(allowed);
  }
  win.webContents.send("app:drawing", drawingState());
}
// The themes and the one in use, as the page last said (src/themes-ui.js),
// for the Theme menus in the tray and the macOS menu bar (theme-menu.js).
let themeMenu = null;
let appMenu = null;

// XenoNet Dev's service keeps its layout and search index apart from the
// installed XenoNet's, starting from a copy of its layout. It also relays the
// control channel (XENONET_CONTROL). The demo instances keep everything in
// their made-up world.
function serverEnv() {
  // XENONET_EXEC: this app, which agents' XenoNet tools run on, as Node (see
  // core/engine/connect.js).
  const env = { ...process.env, XENONET_PORT: String(PORT), XENONET_EXEC: process.execPath, XENONET_NAME: app.getName() };
  delete env.XENONET_CONTROL;
  if (!fromSource) return env;
  env.XENONET_CONTROL = "1";
  if (instance.frozen) env.XENONET_HOT = "0";
  if (instance.demo) {
    env.XENONET_DEMO = "1";
    return env;
  }
  env.XENONET_STATE_DIR ??= appDir("state", { name: instance.name });
  env.XENONET_CACHE_DIR ??= appDir("cache", { name: instance.name });
  const own = path.join(env.XENONET_STATE_DIR, "state.json");
  const installed = path.join(appDir("state"), "state.json");
  if (!fs.existsSync(own) && fs.existsSync(installed)) {
    fs.mkdirSync(env.XENONET_STATE_DIR, { recursive: true });
    fs.copyFileSync(installed, own);
  }
  return env;
}

// An app's link (a session in Claude desktop, ChatGPT or VS Code), a folder
// or a page, which the server asks to have opened from here: Windows lets
// what the app you're using opens come to the front, and not what a
// background process opens. The server builds these from what it knows,
// never from the page; still, only apps' links and absolute paths open.
const APP_LINK = /^(claude|codex|vscode):\/\//i;
async function openTarget(target) {
  if (typeof target === "string" && APP_LINK.test(target)) return void (await shell.openExternal(target));
  if (typeof target === "string" && path.isAbsolute(target)) {
    const error = await shell.openPath(target);
    if (error) throw new Error(error);
    return;
  }
  throw new Error("XenoNet opens only apps' links, folders and pages.");
}

// The service this Desktop runs, the environment it runs it in, and its
// pid when it's XenoNet Dev's own to stop (see stop()).
let serviceBin = null;
let serviceEnv = null;
let ownService = null;

// Starts the service, or finds it running, and connects to it. The service
// is the xenonet binary (service-runtime.js): the installed Service's in a
// packaged XenoNet, the checkout's own build in XenoNet Dev.
async function startServer() {
  const env = serverEnv();
  env.XENONET_RUNTIME_DIR = process.env.XENONET_RUNTIME_DIR || runtimeDir();
  let serviceRoot = process.env.XENONET_SERVICE_ROOT || root;
  if (app.isPackaged) {
    const bundle = path.join(process.resourcesPath, "components");
    if (fs.existsSync(path.join(bundle, "components.json"))) await installBundle(bundle, { env, command: !packagedTest });
    if (!currentComponent("service", env) || !currentComponent("tools", env)) {
      if (packagedTest) throw new Error("The test bundle is missing its components.");
      const available = await componentUpdates({ env });
      for (const component of ["tools", "service"]) {
        if (currentComponent(component, env)) continue;
        const item = available.find((p) => p.component === component);
        if (!item) throw new Error("Install the XenoNet bundle to add its background service and tools.");
        await updateComponent(item, { env });
      }
    }
    await recoverServiceUpdate({ env });
    const installed = installedRuntime(env);
    serviceRoot = installed.root;
    Object.assign(env, installed.env);
  }
  serviceBin = rustBinary({ root: fromSource ? root : null, serviceRoot });
  if (!serviceBin) throw new Error(missingBinary({ fromSource }));
  serviceEnv = env;
  const how = await ensureService({ root: serviceRoot, exec: serviceBin, args: ["serve"], env });
  serviceClient = await desktopClient({ env, handle: (method, body) => {
    if (method === "open") return openTarget(body?.target);
    if (method !== "console") throw new Error("Unsupported Desktop request.");
    return control.handle(body);
  } });
  windowKey = serviceClient.info.windowKey;
  if (fromSource && how === "started") ownService = serviceClient.info.pid;
  serviceClient.on("notification", notify);
  serviceClient.on("connected", (info) => { windowKey = info.windowKey; if (win && !win.isDestroyed()) win.webContents.reload(); });
  serviceClient.on("disconnect", () => { windowKey = null; control?.log("service", "warning", "XenoNet Service disconnected."); });
  // Only the window's own API requests carry this through Desktop, which
  // the service takes for the window's (electron/desktop-server.js).
  const secret = crypto.randomBytes(32).toString("hex");
  frontendServer = await desktopServer({ root, port: Number(process.env.XENONET_DESKTOP_PORT || PORT + 1000), service: SERVICE_BASE, secret, dev: dev ? { hot: !instance?.frozen } : null });
  BASE = frontendServer.base;
  pageHeaders(session.defaultSession, { base: BASE, secret });
  const dir = runtimeDir(env);
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, "desktop.json"), JSON.stringify({ command: process.execPath, args: fromSource ? [root, "--prod"] : [], cwd: root }));
  return how;
}

// XenoNet Dev picks up edits as they're saved. The page hot-reloads through
// Vite (desktop-server.js), and a change to the preload reloads it. A change
// to the rest of electron/, or to the core/ and server/ modules Desktop runs
// itself, relaunches XenoNet Dev.app; a terminal run says to restart
// instead, since a relaunch would leave the terminal behind. The service is
// the checkout's xenonet binary: when a build replaces it (npm run
// rust:bin), XenoNet Dev stops the old service and relaunches onto the new
// one.
function watchSource() {
  const changed = new Set();
  let timer = null;
  const apply = () => {
    const files = [...changed];
    changed.clear();
    const shell = files.filter((f) => !f.endsWith("preload.cjs"));
    if (shell.length) return relaunch(`${shell.join(", ")} changed`);
    if (files.length) win?.webContents.reloadIgnoringCache();
  };
  for (const dir of ["core", "server", "electron"]) {
    fs.watch(path.join(root, dir), { recursive: true }, (_event, file) => {
      if (!/\.[cm]?js$/.test(file ?? "")) return;
      changed.add(path.join(dir, file));
      clearTimeout(timer);
      timer = setTimeout(apply, 200);
    });
  }
  // A build writes the binary over a while, so it's taken once it has held
  // still for a moment.
  let rebuilt = null;
  if (serviceBin) fs.watchFile(serviceBin, { interval: 1000 }, (now, before) => {
    if (!now.size || now.mtimeMs === before.mtimeMs) return;
    clearTimeout(rebuilt);
    rebuilt = setTimeout(() => restartService().catch((err) => console.error(`[xenonet] the service didn't restart: ${err.message}`)), 1500);
  });
}

function relaunch(why) {
  if (!devBundle) return console.log(`[xenonet] ${why}; restart npm run app to load it`);
  console.log(`[xenonet] ${why}, relaunching`);
  // It comes back without taking focus from your editor. It exits rather
  // than quits: a quit waits for the page to agree to close, which a busy
  // page can put off for good, leaving the old copy running.
  process.env.XENONET_SHOW ||= "inactive";
  app.relaunch();
  stop();
  app.exit(0);
}

// Stops the service XenoNet Dev runs, once its binary is rebuilt, so the
// next start runs the new one rather than finding the old one running.
async function restartService() {
  try {
    const c = await connectService({ env: serviceEnv });
    try { await c.request("stop"); } finally { c.close(); }
  } catch {}
  for (const until = Date.now() + 15000; Date.now() < until; await new Promise((r) => setTimeout(r, 200))) {
    try { (await connectService({ env: serviceEnv })).close(); } catch { return relaunch("the xenonet binary changed"); }
  }
  throw new Error("the old service is still running");
}

// macOS: the Dock icon comes and goes with the window, as the taskbar button
// does on Windows; the menu bar item stays either way.
function dockFollowsWindow(visible) {
  if (!isMac || offscreen) return;
  if (visible) app.dock?.show();
  else app.dock?.hide();
}

// `focus` "inactive" brings the window up front to watch without taking
// focus from the app you're in. Nothing shows once XenoNet is quitting: a
// launch handed to a copy on its way out (second-instance), a Dock click
// or a notification would make a new window for its closed one, and load
// it from the server stop() just ended.
function showWindow(focus = true) {
  if (quitting) return;
  if (!win || win.isDestroyed()) return createWindow();
  if (win.isMinimized()) win.restore();
  if (offscreen) return;
  dockFollowsWindow(true);
  if (focus === "inactive") {
    win.showInactive();
    win.moveTop();
  } else {
    win.show();
    if (focus) {
      if (isMac) app.focus({ steal: true });
      win.focus();
    }
  }
  // Said here as well as on the window's show event, which macOS doesn't
  // always send (see hideWindow).
  tellWindow();
}

// A hidden window (XENONET_SHOW=hidden) brought out to watch, at the control
// channel's request: from here on it's an ordinary window.
function reveal() {
  if (!offscreen || !win || win.isDestroyed()) return;
  offscreen = false;
  tellDrawing();
  win.webContents.setAudioMuted(MUTED);
  if (!isMac) {
    win.center();
    win.setSkipTaskbar(false);
  }
}

function hideWindow() {
  if (!win || win.isDestroyed()) return;
  // A full-screen window has to leave its Space first, or macOS leaves a black
  // screen behind.
  if (isMac && win.isFullScreen()) {
    win.once("leave-full-screen", () => hideWindow());
    win.setFullScreen(false);
    return;
  }
  win.hide();
  dockFollowsWindow(false);
  // On macOS the window's hide event follows whether it can be seen, so a
  // window hidden while other windows covered it sends none: said here,
  // for the page, which stops drawing (src/render-budget.js), and for you.
  tellWindow();
}
showWindow.isFocused = () => !!win && !win.isDestroyed() && win.isFocused() && win.isVisible();

// The theme's colors for the window itself (see app:theme): its background
// shows behind the page while it loads and as the window resizes, and on
// Windows the caption buttons sit over the page's top strip. Kept from the
// last theme, so the window opens in it.
const HEX = /^#[0-9a-f]{6}$/i;
function look() {
  const kept = prefs().get("windowTheme");
  return { background: HEX.test(kept?.background) ? kept.background : "#05070a", symbols: HEX.test(kept?.symbols) ? kept.symbols : "#5fe1ff" };
}

// Windows draws its caption buttons over the page's 32 px top strip; macOS
// keeps its traffic lights and centres them in the same strip. On a small
// Mac screen the window starts inside the visible area.
function chrome() {
  const { background, symbols } = look();
  if (!isMac) return { width: 1600, height: 960, minWidth: 1100, minHeight: 700, titleBarStyle: "hidden", titleBarOverlay: { color: background, symbolColor: symbols, height: 32 } };
  const { width, height } = screen.getPrimaryDisplay().workAreaSize;
  return {
    width: Math.min(1600, width - 48),
    height: Math.min(960, height - 32),
    minWidth: Math.min(1100, width),
    minHeight: Math.min(700, height),
    titleBarStyle: "hidden",
    trafficLightPosition: { x: 14, y: 10 },
  };
}

async function createWindow() {
  const icon = await iconImage(256);
  win = new BrowserWindow({
    ...chrome(),
    // Windows renders a hidden window only once it's been shown, so a hidden
    // one is shown off the screen. On macOS it's simply never shown.
    ...(offscreen && !isMac ? { x: -6000, y: 40, skipTaskbar: true } : {}),
    show: false,
    title: "XenoNet",
    icon,
    backgroundColor: look().background,
    webPreferences: {
      preload: path.join(root, "electron", "preload.cjs"),
      contextIsolation: true,
      sandbox: true,
      // See tellDrawing: off only where the page must draw unseen.
      backgroundThrottling: !offscreen,
      spellcheck: false,
    },
  });
  throttling = !offscreen;
  control?.watch(win);
  if (offscreen || MUTED) win.webContents.setAudioMuted(true);
  win.webContents.setWindowOpenHandler(({ url }) => {
    if (/^https?:\/\//i.test(url)) shell.openExternal(url);
    return { action: "deny" };
  });
  // Stay on our own origin, compared as an origin (see trust.js); anything
  // else opens in the browser.
  win.webContents.on("will-navigate", (e, url) => {
    if (!sameOrigin(url, BASE)) {
      e.preventDefault();
      if (/^https?:\/\//i.test(url)) shell.openExternal(url);
    }
  });
  // A load that fails (the server mid-restart, say) tries again rather than
  // leaving the window on an error page.
  win.webContents.on("render-process-gone", (_e, d) => d.reason !== "clean-exit" && desktopLog.error("desktop.page-gone", `the page's process ended: ${d.reason}${d.exitCode ? ` (exit code ${d.exitCode})` : ""}`, { fix: "XenoNet reloads it; if it keeps happening, xenonet diagnose says more." }));
  win.webContents.on("unresponsive", () => desktopLog.warn("desktop.page-unresponsive", "the page stopped responding"));
  win.webContents.on("did-fail-load", (_e, code, _desc, url, isMainFrame) => {
    if (!isMainFrame || code === -3 || quitting) return; // -3: superseded by another load
    console.log(`[xenonet] couldn't load ${url} (${code}), retrying`);
    desktopLog.warn("desktop.load-failed", `couldn't load the page (${code}); trying again`, { fix: "XenoNet's service may be restarting; if it stays, xenonet service status says why." });
    setTimeout(() => win && !win.isDestroyed() && win.loadURL(`${BASE}/`).catch(() => {}), 1000);
  });
  // A page that loaded but never started stays blank: in XenoNet Dev, a
  // module that didn't load because the server restarted as Vite reloaded
  // the page, say. Once it's had BOOT_MS to start (its driver is the last
  // thing it sets up), it's loaded again, longer apart each time, five times
  // in a row at most.
  let bootTimer = null;
  let stuck = 0;
  win.webContents.on("did-finish-load", () => {
    clearTimeout(bootTimer);
    bootTimer = setTimeout(async () => {
      if (!win || win.isDestroyed() || quitting || win.webContents.isLoading()) return;
      const started = await win.webContents.executeJavaScript("!!window.__xenonet?.driver", true).catch(() => false);
      if (started) return void (stuck = 0);
      if (stuck >= 5) return desktopLog.error("desktop.page-stuck", "the page loaded but didn't start, five times in a row", { fix: "xenonet diagnose says what failed; restarting XenoNet may clear it." });
      stuck += 1;
      console.log("[xenonet] the page loaded but didn't start; loading it again");
      desktopLog.warn("desktop.page-stuck", "the page loaded but didn't start; loading it again", { fix: "If it keeps happening, xenonet diagnose says what failed." });
      win.webContents.reloadIgnoringCache();
    }, BOOT_MS * (stuck + 1));
  });
  win.once("ready-to-show", () => {
    if (offscreen) {
      if (!isMac) win.showInactive();
    } else if (startHidden) {
      /* in the tray until asked for */
    } else if (SHOW === "inactive") showWindow("inactive");
    else win.show();
  });
  win.on("focus", withdrawNotes);
  for (const event of ["show", "hide", "minimize", "restore"]) win.on(event, () => tellWindow());
  // Closing the window keeps XenoNet in the tray; Quit is in the tray menu.
  win.on("close", (e) => {
    if (quitting) return;
    e.preventDefault();
    hideWindow();
    // Once ever, the first time you close the window.
    const p = prefs();
    if (!p.get("trayHint")) {
      p.set("trayHint", true);
      const body = isMac ? "It keeps running in the menu bar. To quit, choose Quit XenoNet from its menu bar icon." : "It keeps running in the tray. To quit, use Quit XenoNet in the tray menu.";
      new Notification({ title: "XenoNet is still running", body }).show();
    }
  });
  tellWindow();
  await win.loadURL(`${BASE}/`).catch(() => {}); // did-fail-load retries
  return win;
}

// Whether an IPC call comes from XenoNet's own page: the main frame of the
// window, on the server's origin, not a frame or a page navigated
// elsewhere. Every handler of the bridge asks it (see trust.js).
const ownPage = (e) => fromOwnPage(e, { win, base: BASE });

// Posters go to Pictures\XenoNet and replay clips to Videos\XenoNet
// (the system's own folders: Known Folders on Windows, ~/Pictures and
// ~/Movies on macOS, XDG user dirs on Linux). XENONET_CAPTURES_DIR puts both in
// one folder instead.
function capturesDir(filename) {
  if (process.env.XENONET_CAPTURES_DIR) return process.env.XENONET_CAPTURES_DIR;
  return path.join(app.getPath(/\.(webm|mp4)$/i.test(filename) ? "videos" : "pictures"), "XenoNet");
}

function wireDownloads() {
  session.defaultSession.on("will-download", (_e, item) => {
    const dir = capturesDir(item.getFilename());
    fs.mkdirSync(dir, { recursive: true });
    const file = path.join(dir, item.getFilename());
    item.setSavePath(file);
    item.once("done", (_ev, state) => {
      if (state !== "completed") return;
      const where = path.relative(app.getPath("home"), dir).startsWith("..") ? dir : path.relative(app.getPath("home"), dir);
      const n = new Notification({ title: `Saved to ${where}`, body: path.basename(file) });
      n.on("click", () => shell.showItemInFolder(file));
      n.show();
    });
  });
}

function wireIpc() {
  ipcMain.handle("app:updates", async (e, action, value) => {
    if (!ownPage(e)) return null;
    if (action === "components") return app.isPackaged && !packagedTest ? componentUpdates() : [];
    if (action === "component-update") {
      if (!app.isPackaged || packagedTest || !["service", "tools"].includes(value)) return null;
      const item = (await componentUpdates()).find((x) => x.component === value);
      if (!item) throw new Error("This component has no published update.");
      return updateComponent(item);
    }
    return updates?.act(action, value);
  });
  // ⌘W with no terminal tab focused (macOS).
  ipcMain.on("app:hide", (e) => ownPage(e) && win?.close());
  // The window key, for the page's reports of you (src/you.js): only to
  // XenoNet's own page, and null while it isn't connected to the service.
  ipcMain.handle("app:window-key", (e) => (ownPage(e) ? windowKey : null));
  // Whether the window shows, for the same reports: "shown" or "hidden",
  // asked when the page starts and then told (tellWindow).
  ipcMain.handle("app:window-state", (e) => (ownPage(e) ? windowState() : null));
  ipcMain.handle("app:save-patch", async (e, value = {}) => {
    if (!ownPage(e)) return null;
    const exported = patchExport(value); if (!exported) return null;
    const { dialog } = await import("electron");
    const parent = win && !win.isDestroyed() ? win : null;
    const options = { title: "Export the reviewed patch", buttonLabel: "Export", defaultPath: path.join(app.getPath("documents"), exported.filename), filters: [{ name: "Git patch", extensions: ["patch"] }], properties: ["createDirectory", "showOverwriteConfirmation"] };
    const chosen = parent ? await dialog.showSaveDialog(parent, options) : await dialog.showSaveDialog(options);
    if (chosen.canceled || !chosen.filePath) return null;
    // The receipt describes the cached revision. It contains no task text,
    // source path, or credentials; patch contents are saved exactly as read.
    fs.writeFileSync(chosen.filePath, exported.content, { mode: 0o600 });
    return { file: tilde(chosen.filePath, app.getPath("home")) };
  });
  // What the page's pace of drawing depends on that only this process
  // knows (see tellDrawing).
  ipcMain.handle("app:drawing", (e) => (ownPage(e) ? drawingState() : null));
  // An actor's token saved to a file you pick (the Actors panel's "Save to
  // a file…"; docs/design/actors/interface.md, section 2): the save dialog
  // over the window, then the file written so only you can read it (0600),
  // and the token leaves the page. Only from XenoNet's own page, and only a
  // token. The page gets where it went (with ~) and a warning for a file in
  // a git working tree, where agents read freely; null when you cancel.
  ipcMain.handle("app:save-token", async (e, { token, slug } = {}) => {
    if (!ownPage(e) || !isToken(token)) return null;
    const { dialog } = await import("electron");
    const home = app.getPath("home");
    const name = typeof slug === "string" && /^[a-z0-9][a-z0-9-]{0,31}$/.test(slug) ? slug : "actor";
    const options = {
      title: "Save the actor's token",
      buttonLabel: "Save",
      message: "Only you will be able to read this file. Keep it out of your projects and repos, where agents read freely.",
      defaultPath: path.join(home, ".config", "xenonet", `${name}.token`),
      properties: ["createDirectory", "showOverwriteConfirmation"],
    };
    const parent = win && !win.isDestroyed() ? win : null;
    const res = parent ? await dialog.showSaveDialog(parent, options) : await dialog.showSaveDialog(options);
    if (res.canceled || !res.filePath) return null;
    // You chose it, and the dialog asked before replacing a file.
    const file = writeTokenFile(res.filePath, token, { force: true });
    const tree = gitTreeOf(path.dirname(file));
    return { file: tilde(file, home), warning: tree ? `It's inside a git working tree (${tilde(tree, home)}), where agents read freely and tokens get committed.` : null };
  });
  // Native notifications for the page's alerts; the page decides what's new.
  // One per alert: a newer one replaces it, and it's withdrawn from the
  // notification center once the alert resolves or the window comes to the
  // front, where the strip shows it. Behind the window the notification
  // carries the alert's sound; in front, the page plays its own.
  ipcMain.on("app:notify", (e, n) => ownPage(e) && notify(n ?? {}));
  ipcMain.on("app:notify-clear", (e, keys) => {
    if (!ownPage(e)) return;
    for (const key of Array.isArray(keys) ? keys : []) {
      notes.get(key)?.close();
      notes.delete(key);
    }
  });
  // Films of the day (src/film.js): the window, recorded to the videos
  // folder in the frame the page names (a story, 9:16, unless it says
  // wide); the page says when to start and stop, never where.
  ipcMain.handle("app:record", (e, { action, frame } = {}) => (!ownPage(e) ? null : action === "stop" ? recorder.stop() : recorder.start({ file: filmFile(), ...filmFrame(frame) })));
  // Choosing the projects folder (src/root.js): the native folder picker,
  // over the window. The page gets the folder picked, or null, and the
  // server checks it before XenoNet reads it.
  ipcMain.handle("app:choose-folder", async (e, { defaultPath } = {}) => {
    if (!ownPage(e)) return null;
    const { dialog } = await import("electron");
    const options = {
      title: "Choose your projects folder",
      buttonLabel: "Choose",
      message: "The folder that holds your projects. XenoNet checks it before it reads it.",
      ...(typeof defaultPath === "string" && path.isAbsolute(defaultPath) ? { defaultPath } : {}),
      // An alias stays as you picked it, the way your shell and the apps
      // spell the folders sessions run in.
      properties: ["openDirectory", "createDirectory", "noResolveAliases"],
    };
    const parent = win && !win.isDestroyed() ? win : null;
    const res = parent ? await dialog.showOpenDialog(parent, options) : await dialog.showOpenDialog(options);
    return res.canceled ? null : (res.filePaths[0] ?? null);
  });

  // The page's theme changed (src/themes-ui.js): the window takes its
  // colors, and keeps them for next time, unless it's a theme the page is
  // only looking at (keep: false). Only plain #rrggbb colors.
  ipcMain.on("app:theme", (e, { background, symbols, keep = true } = {}) => {
    if (!ownPage(e) || !HEX.test(background ?? "") || !HEX.test(symbols ?? "") || !win || win.isDestroyed()) return;
    win.setBackgroundColor(background);
    if (!isMac) win.setTitleBarOverlay({ color: background, symbolColor: symbols, height: 32 });
    const kept = prefs().get("windowTheme");
    if (keep !== false && (kept?.background !== background || kept?.symbols !== symbols)) prefs().set("windowTheme", { background, symbols });
  });

  // The page's themes, whenever they or the one in use change: the Theme
  // menus list them, and a click runs the page's own command for one, as
  // the palette does. Only from XenoNet's own page, and only what fits.
  ipcMain.on("app:themes", (e, said) => {
    if (!ownPage(e)) return;
    themeMenu = themeMenuOf(said);
    appMenu?.setThemes(themeMenu);
    globalThis.__xenonetTray?.setThemes(themeMenu);
  });
}

// Recording the window (films of the day), shared by the page and the
// control channel.
const recorder = createRecorder(() => win, { onChange: () => tellDrawing() });
// The frame a film records in (core/frames.js): the window takes the
// largest size in the frame's shape that fits its display, and the film is
// scaled to the frame's own width.
function filmFrame(frame) {
  const f = FILM_FRAMES[frame] ?? FILM_FRAMES[DEFAULT_FRAME];
  const display = win && !win.isDestroyed() ? screen.getDisplayMatching(win.getBounds()) : screen.getPrimaryDisplay();
  const scale = display.scaleFactor || 1;
  const size = fitFrame(f, { width: display.workAreaSize.width * scale, height: display.workAreaSize.height * scale });
  return { size: [size.width, size.height], width: f.width };
}
function filmFile() {
  const day = new Date().toLocaleDateString("en-CA");
  const dir = capturesDir("film.mp4");
  let file = path.join(dir, `xenonet-${day}.mp4`);
  for (let i = 2; fs.existsSync(file); i++) file = path.join(dir, `xenonet-${day}-${i}.mp4`);
  return file;
}

const notes = new Map();
// One notification per key: a newer one replaces it. A click opens XenoNet on
// the session it's about (`id`) or runs a command (`command`, such as the
// doctor's panel).
function notify({ key, id, command, title, subtitle, body, sound }) {
  if (!key || showWindow.isFocused() || offscreen) return;
  notes.get(key)?.close();
  const clip = (s, n) => String(s ?? "").slice(0, n);
  // macOS shows a subtitle line; elsewhere it leads the body.
  const n = new Notification({
    title: clip(title, 120),
    ...(isMac ? { subtitle: clip(subtitle, 120), body: clip(body, 240) } : { body: clip([subtitle, body].filter(Boolean).join(" · "), 240) }),
    silent: !sound,
  });
  n.on("click", () => {
    showWindow();
    if (command) win?.webContents.send("app:command", command);
    else win?.webContents.send("app:select", id);
  });
  n.on("close", () => notes.get(key) === n && notes.delete(key));
  notes.set(key, n);
  n.show();
  // The Dock icon bounces once; the taskbar button flashes until you look.
  if (isMac) app.dock?.bounce("informational");
  else if (win?.isVisible()) win.flashFrame(true);
}
function withdrawNotes() {
  for (const n of notes.values()) n.close();
  notes.clear();
  win?.flashFrame(false);
}

if (iconsArg >= 0) {
  app.whenReady().then(async () => {
    const dir = path.resolve(process.argv[iconsArg + 1] ?? path.join(root, "build"));
    fs.mkdirSync(dir, { recursive: true });
    const png = async (size) => Buffer.from((await iconDataUrl(size)).split(",")[1], "base64");
    const sizes = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256];
    const entries = [];
    for (const size of sizes) entries.push({ size, png: await png(size) });
    fs.writeFileSync(path.join(dir, "icon.ico"), packIco(entries));
    fs.writeFileSync(path.join(dir, "icon.png"), await png(1024));
    fs.writeFileSync(path.join(dir, "icon-sheet.png"), Buffer.from((await sheetDataUrl([16, 24, 32, 48, 64, 128, 256])).split(",")[1], "base64"));
    for (const [kind, name] of [["sidebar", "installerSidebar"], ["header", "installerHeader"]]) {
      const art = Buffer.from((await artDataUrl(kind)).split(",")[1], "base64");
      fs.writeFileSync(path.join(dir, `${name}.png`), art);
      fs.writeFileSync(path.join(dir, `${name}.bmp`), bmpFromImage(nativeImage.createFromBuffer(art)));
    }
    // macOS: the icon on Apple's grid, the disk image's background at 1x and
    // 2x, and the installer package's side art (2x, marked 144 dpi).
    const macPngs = {};
    for (const size of [16, 32, 64, 128, 256, 512, 1024]) macPngs[size] = Buffer.from((await iconDataUrl(size, 0, 0, { mac: true })).split(",")[1], "base64");
    fs.writeFileSync(path.join(dir, "icon.icns"), packIcns(macPngs));
    fs.writeFileSync(path.join(dir, "icon-mac.png"), macPngs[1024]);
    for (const scale of [1, 2]) fs.writeFileSync(path.join(dir, `dmgBackground${scale === 2 ? "@2x" : ""}.png`), Buffer.from((await dmgArtDataUrl(scale)).split(",")[1], "base64"));
    fs.writeFileSync(path.join(dir, "pkgBackground.png"), pngWithDpi(Buffer.from((await pkgArtDataUrl(2)).split(",")[1], "base64"), 144));
    console.log(`icons written to ${dir}: icon.ico (${sizes.join(", ")}), icon.png (1024), icon-sheet.png, installerSidebar.bmp, installerHeader.bmp, icon.icns, dmgBackground.png (1x, 2x), pkgBackground.png`);
    closePainter();
    app.exit(0);
  });
} else if (shortcutArg >= 0) {
  app.whenReady().then(() => {
    if (process.platform !== "win32") {
      console.log("--install-shortcut makes a Windows shortcut; on macOS, drag XenoNet to the Dock instead");
      return app.exit(1);
    }
    // Point at the packaged exe; the shortcut's app ID makes notifications and
    // taskbar pins group under XenoNet.
    const exe = process.execPath;
    const link = path.join(app.getPath("desktop"), "XenoNet.lnk");
    const ok = shell.writeShortcutLink(link, fs.existsSync(link) ? "replace" : "create", {
      target: exe,
      cwd: path.dirname(exe),
      description: "A live console for your projects and the sessions working in them",
      icon: exe,
      iconIndex: 0,
      appUserModelId: APP_ID,
    });
    console.log(ok ? `shortcut written: ${link}` : `could not write ${link}`);
    app.exit(ok ? 0 : 1);
  });
} else if (iconArg >= 0) {
  app.whenReady().then(async () => {
    const out = path.resolve(process.argv[iconArg + 1] ?? path.join(root, "build", "icon.png"));
    fs.mkdirSync(path.dirname(out), { recursive: true });
    // `--badge <working>,<needs>` renders a tray state instead of the app icon.
    const badge = process.argv.indexOf("--badge");
    const [working, need] = badge > 0 ? process.argv[badge + 1].split(",").map(Number) : [0, 0];
    const url = await iconDataUrl(badge > 0 ? 64 : 512, working, need);
    fs.writeFileSync(out, Buffer.from(url.split(",")[1], "base64"));
    console.log(`icon written: ${out}`);
    closePainter();
    app.exit(0);
  });
} else {
  const uninstallFrom = (argv) => uninstall({ ask: false, purge: argv.includes("--purge"), stop });
  app.on("second-instance", (_e, argv) => (isMac && argv.includes("--uninstall") ? uninstallFrom(argv) : showWindow()));
  app.on("before-quit", stop);
  // `pkill XenoNet`, and the installer package closing a running copy, quit
  // cleanly instead of being ignored.
  process.on("SIGTERM", () => app.quit());
  app.on("window-all-closed", () => {
    /* keep running in the tray */
  });
  // macOS: clicking the Dock icon, or opening XenoNet again from Finder (once
  // the window exists; startup makes its own).
  app.on("activate", () => win && showWindow());

  app.whenReady().then(async () => {
    if (!gotLock) return;
    if (uninstallMode) return uninstallFrom(process.argv);
    if (process.platform === "win32" && !offscreen && !packagedTest) {
      const activator = windowsToastActivator({ productName: app.isPackaged ? "XenoNet" : "Electron", fallback: toastActivator, readShortcut: (file) => shell.readShortcutLink(file) });
      app.setToastActivatorCLSID(activator);
      // IconUri accepts an image file, not an executable's icon resource.
      const icon = path.join(app.getPath("userData"), "notification-icon.png");
      const bytes = Buffer.from((await iconDataUrl(64)).split(",")[1], "base64");
      fs.mkdirSync(path.dirname(icon), { recursive: true });
      if (!fs.existsSync(icon) || !fs.readFileSync(icon).equals(bytes)) fs.writeFileSync(icon, bytes);
      registerWindowsIdentity({ id: APP_ID, activator, icon });
    }
    startHidden ||= launchedAtLogin();
    if (isMac) {
      if (startHidden || offscreen) app.dock?.hide();
      if (!offscreen && !startHidden && (await offerMove())) return;
      appMenu = installMenu({ send: (cmd) => (showWindow(), win?.webContents.send("app:command", cmd)), dev, uninstall: () => uninstall({ win, stop }) });
      if (!app.isPackaged) app.dock?.setIcon(nativeImage.createFromDataURL(await iconDataUrl(512, 0, 0, { mac: true })));
    }
    const { default: updaterModule } = await import("electron-updater");
    const metadata = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));
    updates = createUpdates({
      updater: updaterModule.autoUpdater,
      enabled: app.isPackaged && !packagedTest && ["win32", "darwin"].includes(process.platform),
      automaticInstall: !isMac || metadata.xenonetMacAutoUpdate === true,
      checkComponents: app.isPackaged && !packagedTest ? () => componentUpdates() : null,
      version: app.getVersion(), preferences: prefs(),
      changed: (state) => win?.webContents.send("app:update-state", state),
      available: ({ version }) => {
        const note = new Notification({ title: `XenoNet ${version} is available`, body: "Open Updates for the new download." });
        note.on("click", () => { showWindow(); win?.webContents.send("app:command", "updates"); });
        note.show();
      },
      ready: () => {
        const note = new Notification({ title: "XenoNet is ready to update", body: "Open Updates to restart Desktop. Your terminals keep running." });
        note.on("click", () => { showWindow(); win?.webContents.send("app:command", "updates"); });
        note.show();
      },
      openRelease: () => shell.openExternal("https://github.com/willctl/xenonet/releases/latest"),
      confirmRestart: async () => {
        const { dialog } = await import("electron");
        const result = await dialog.showMessageBox(win, { type: "question", message: "Restart XenoNet and install the update?", detail: "Desktop will restart. Your terminals keep running, and sessions stay connected to XenoNet.", buttons: ["Restart and update", "Later"], defaultId: 1, cancelId: 1 });
        return result.response === 0;
      },
    });
    await loadShellEnv({ locale: app.getSystemLocale() });
    wireIpc();
    wireDownloads();
    control = createControl({ root: fromSource ? root : runtimeDir(), production: !fromSource, instance: instance || { id: "installed", name: "XenoNet", demo: false }, port: PORT, getWindow: () => win, showWindow, hideWindow, reveal, recorder, filmFile, onDriven: (on) => ((driven = on), tellDrawing()) });
    // A locked screen or a computer going to sleep: nothing to draw for.
    for (const [event, key, on] of [["lock-screen", "locked", true], ["unlock-screen", "locked", false], ["suspend", "asleep", true], ["resume", "asleep", false]]) powerMonitor.on(event, () => ((held[key] = on), tellDrawing()));
    // The screen may be locked already, with no lock-screen event to come:
    // XenoNet started by a script or an agent while you're away. On macOS
    // Electron says only "idle" then, so the session's own word is read.
    if (isMac)
      execFile("/usr/sbin/ioreg", ["-n", "Root", "-d1", "-a"], { timeout: 3000 }, (err, out) => {
        if (!err && /<key>CGSSessionScreenIsLocked<\/key>\s*<true\/>/.test(out)) (held.locked = true), tellDrawing();
      });
    else if (powerMonitor.getSystemIdleState(1) === "locked") held.locked = true;
    try {
      const how = await startServer();
      console.log(`[xenonet] server ${how} at ${BASE}`);
      // The control channel comes through a server this app started.
      if (control && how === "reused") console.log(`[xenonet] the server at ${BASE} was already running, so scripts and agents can't control this window; stop that server and restart ${app.name}`);
    } catch (err) {
      const { dialog } = await import("electron");
      dialog.showErrorBox("XenoNet couldn't start its server", String(err.message ?? err));
      app.exit(1);
      return;
    }
    pty = registerServicePty({ allowed: ownPage, client: serviceClient, getWindow: () => win });
    await createWindow();
    updates.start();
    // A frozen instance (test, film) runs the code as it was at launch, so an
    // edit can't restart anything under the end-to-end tests or a film.
    if (dev && !instance?.frozen) watchSource();
    // A hidden instance (tests) stays out of the menu bar and the tray.
    if (SHOW !== "hidden") {
      const tray = createTray({
        // The service itself: Desktop's frontend takes only the page's.
        baseUrl: SERVICE_BASE,
        showWindow,
        sendCommand: (cmd) => win?.webContents.send("app:command", cmd),
        select: (id) => win?.webContents.send("app:select", id),
        themes: themeMenu,
      });
      // Before it starts, so the page's themes reach it from now on.
      globalThis.__xenonetTray = tray;
      await tray.start();
    }
    if (isMac && !startHidden && !offscreen) askAboutLogin(win);
  });
}
