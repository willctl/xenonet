import { test } from "node:test";
import assert from "node:assert/strict";
import { runPrivateCommand, protectFile, userSid } from "../core/private-file.js";

test("private commands tolerate Windows inspection delays and still time out", () => {
  const slowLookup = (file, args, options) => {
    assert.equal(file, "whoami");
    assert.deepEqual(args, ["/user", "/fo", "csv", "/nh"]);
    assert.equal(options.windowsHide, true);
    if (options.timeout < 13000) throw Object.assign(new Error("timed out"), { code: "ETIMEDOUT" });
    assert.ok(options.timeout <= 60000, "the wait remains bounded");
    return '"account","S-1-5-21-123"';
  };
  assert.equal(userSid({ run: (file, args) => runPrivateCommand(file, args, { exec: slowLookup }) }), "S-1-5-21-123");
});

test("failed account lookup never proceeds to a credential-file ACL write", () => {
  const calls = [];
  const failure = Object.assign(new Error("timed out"), { code: "ETIMEDOUT" });
  const run = (file, args) => runPrivateCommand(file, args, { exec: () => { calls.push(file); throw failure; } });
  assert.throws(() => protectFile("credential.json", { platform: "win32", run }), (error) => error === failure);
  assert.deepEqual(calls, ["whoami"]);
});

test("successful lookup still grants only the current-user SID and removes inherited permissions", () => {
  const calls = [];
  protectFile("credential.json", { platform: "win32", run: (file, args) => {
    calls.push([file, args]);
    return file === "whoami" ? '"account","S-1-5-21-123"' : "";
  } });
  assert.deepEqual(calls[1], ["icacls", ["credential.json", "/inheritance:r", "/grant:r", "*S-1-5-21-123:F", "/q"]]);
});
