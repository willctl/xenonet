import { test } from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { registerWindowsIdentity, windowsToastActivator } from "../electron/windows-identity.js";

const existingClsid = "{4EED6B67-DC94-4A1B-AFD5-B1965834A1EF}";
const fallback = "{C7A8A779-18B8-4DF9-BAF8-7BF6C86845EC}";

test("Windows notification identity says XenoNet for installed and source apps", () => {
  for (const id of ["dev.xenonet.app", "dev.xenonet.app.dev"]) {
    const calls = [];
    registerWindowsIdentity({ id, icon: "notification-icon.png", activator: existingClsid, platform: "win32", run: (...args) => calls.push(args) });
    assert.equal(calls[0][0], "reg.exe");
    assert.deepEqual(calls[0][1], ["add", `HKCU\\Software\\Classes\\AppUserModelId\\${id}`, "/v", "DisplayName", "/t", "REG_SZ", "/d", "XenoNet", "/f"]);
    assert.equal(calls[1][1][3], "IconUri");
    assert.equal(calls[1][1][7], path.resolve("notification-icon.png"));
    assert.equal(calls[2][1][3], "CustomActivator");
    assert.equal(calls[2][1][7], existingClsid);
    assert.equal(calls[0][2].windowsHide, true);
  }
});

test("notification registration reuses the same shortcut activator as Electron without changing the shortcut", () => {
  for (const productName of ["Electron", "XenoNet"]) {
    const read = [];
    const result = windowsToastActivator({ productName, fallback, programs: "redirected/Programs", readShortcut(file) {
      read.push(file);
      return { toastActivatorClsid: existingClsid, appUserModelId: "an-earlier-id" };
    } });
    assert.equal(result, existingClsid);
    assert.deepEqual(read, [path.join("redirected/Programs", `${productName}.lnk`)]);
  }
});

test("missing or invalid shortcut activators use XenoNet's stable fallback", () => {
  for (const readShortcut of [() => { throw new Error("missing"); }, () => ({}), () => ({ toastActivatorClsid: "invalid" })]) {
    assert.equal(windowsToastActivator({ productName: "Electron", fallback, programs: "Programs", readShortcut }), fallback);
  }
});

test("notification identity resolves Windows' redirected Programs folder", () => {
  const calls = [];
  assert.equal(windowsToastActivator({ productName: "Electron", fallback, run(file, args, options) {
    calls.push({ file, args, options }); return "redirected/Programs\r\n";
  }, readShortcut(file) {
    assert.equal(file, path.join("redirected/Programs", "Electron.lnk"));
    return { toastActivatorClsid: existingClsid };
  } }), existingClsid);
  assert.equal(calls[0].file, "powershell.exe");
  assert.equal(calls[0].options.windowsHide, true);
});
