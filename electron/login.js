// Starting at sign-in, the same setting in the tray, the macOS app menu, the
// Windows installer's Options page and the first-run question on macOS.
//   Windows: a Run entry named after the app ID, with --hidden, which the
//            installer also writes.
//   macOS:   the app as a login item (System Settings › General › Login
//            Items). Those can't carry arguments, so a login launch is
//            recognised with wasOpenedAtLogin instead of --hidden.
import { app, dialog, shell } from "electron";

const isMac = process.platform === "darwin";
const ARGS = ["--hidden"];
const listeners = new Set();

// The tray and the macOS app menu both show this checkbox; each redraws when
// the other changes it.
export const onLoginChange = (fn) => listeners.add(fn);

export const loginSupported = process.platform === "win32" || isMac;
export const loginLabel = isMac ? "Open at Login" : "Start at sign-in";

export function opensAtLogin() {
  if (!loginSupported) return false;
  return app.getLoginItemSettings(isMac ? {} : { args: ARGS }).openAtLogin;
}

export async function setOpensAtLogin(on) {
  if (!loginSupported) return;
  app.setLoginItemSettings(isMac ? { openAtLogin: on } : { openAtLogin: on, args: ARGS });
  for (const fn of listeners) fn(on);
  if (!isMac || !on) return;
  // macOS can hold a new login item until it's allowed in System Settings.
  const { status } = app.getLoginItemSettings();
  if (status !== "requires-approval") return;
  const { response } = await dialog.showMessageBox({
    type: "info",
    message: "Allow XenoNet to open at login",
    detail: "macOS needs your OK first. In System Settings › General › Login Items & Extensions, turn on XenoNet.",
    buttons: ["Open Login Items", "Later"],
    defaultId: 0,
    cancelId: 1,
  });
  if (response === 0) shell.openExternal("x-apple.systempreferences:com.apple.LoginItems-Settings.extension");
}

export function launchedAtLogin() {
  if (process.argv.includes("--hidden")) return true;
  return isMac && app.isPackaged && app.getLoginItemSettings().wasOpenedAtLogin;
}
