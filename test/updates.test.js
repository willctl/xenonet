import { test } from "node:test";
import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { createUpdates } from "../electron/updates.js";

function fixture(options = {}) {
  const updater = new EventEmitter();
  const stored = new Map();
  let installs = 0, downloads = 0, checks = 0, notifications = 0;
  updater.checkForUpdates = async () => { checks++; updater.emit("update-available", { version: "0.6.0" }); };
  updater.downloadUpdate = async () => { downloads++; updater.emit("download-progress", { percent: 42.4 }); updater.emit("update-downloaded", { version: "0.6.0" }); };
  updater.quitAndInstall = () => { installs++; };
  const scheduled = new Set();
  const controller = createUpdates({ updater, enabled: true, automaticInstall: true, version: "0.5.0", preferences: { get: (key) => stored.get(key), set: (key, value) => stored.set(key, value) }, ready: () => notifications++, schedule: (fn) => { scheduled.add(fn); return fn; }, cancel: (fn) => scheduled.delete(fn), confirmRestart: async () => false, openRelease: async () => {}, ...options });
  return { updater, controller, scheduled, counts: () => ({ installs, downloads, checks, notifications }) };
}

test("background update downloads once and waits for an explicit restart", async () => {
  const f = fixture();
  assert.equal(f.updater.autoInstallOnAppQuit, false);
  assert.equal(f.updater.allowPrerelease, false);
  assert.equal(f.updater.allowDowngrade, false);
  assert.equal(f.updater.disableWebInstaller, true);
  await f.controller.act("check");
  assert.equal(f.controller.snapshot().status, "ready");
  await f.controller.act("check");
  await f.controller.act("install");
  assert.deepEqual(f.counts(), { checks: 1, downloads: 1, installs: 0, notifications: 1 });
  f.controller.dispose();
  assert.equal(f.scheduled.size, 0);
  assert.equal(f.updater.listenerCount("error"), 0);
});

test("confirmed restart installs exactly once while its confirmation is open", async () => {
  let confirm;
  const f = fixture({ confirmRestart: () => new Promise((resolve) => confirm = resolve) });
  await f.controller.act("check");
  const pending = f.controller.act("install");
  await f.controller.act("install");
  confirm(true);
  await pending;
  assert.equal(f.counts().installs, 1);
});

test("unsigned Mac builds detect updates without downloading an unusable payload", async () => {
  let notices = 0;
  const f = fixture({ automaticInstall: false, available: () => notices++ });
  await f.controller.act("check");
  await f.controller.act("download");
  await f.controller.act("install");
  assert.equal(f.controller.snapshot().status, "available");
  assert.equal(f.counts().downloads, 0);
  assert.equal(f.counts().installs, 0);
  await f.controller.act("check");
  assert.equal(notices, 1);
});

test("source builds never check, download or install", async () => {
  const f = fixture({ enabled: false });
  f.controller.start();
  for (const action of ["check", "download", "install"]) await f.controller.act(action);
  assert.equal(f.controller.snapshot().status, "development");
  assert.equal(f.scheduled.size, 0);
  assert.equal(f.counts().checks, 0);
});

test("turning off automatic checks still allows a manual check and download", async () => {
  const f = fixture();
  f.controller.start();
  await f.controller.act("automatic", false);
  assert.equal(f.scheduled.size, 0);
  await f.controller.act("check");
  assert.equal(f.controller.snapshot().status, "available");
  assert.equal(f.counts().downloads, 0);
  await f.controller.act("download");
  assert.equal(f.controller.snapshot().status, "ready");
});

test("network failure is retryable and never leaks raw errors to the page", async () => {
  const f = fixture();
  f.updater.checkForUpdates = async () => { throw new Error("private machine path"); };
  await f.controller.act("check");
  assert.equal(f.controller.snapshot().status, "error");
  assert.doesNotMatch(JSON.stringify(f.controller.snapshot()), /private machine/);
  f.updater.checkForUpdates = async () => f.updater.emit("update-not-available");
  await f.controller.act("check");
  assert.equal(f.controller.snapshot().status, "current");
});

test("overlapping checks share one request", async () => {
  const f = fixture();
  let finish, calls = 0;
  f.updater.checkForUpdates = () => { calls++; return new Promise((resolve) => finish = resolve); };
  const first = f.controller.act("check");
  await f.controller.act("check");
  assert.equal(calls, 1);
  finish();
  await first;
});
