// Actors' token files (electron/lib/token-file.js): on a made-up Windows, a
// token file's ACL, read and written. The made-up tokens are
// built here, so no file of XenoNet's holds one.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { readTokenFile, writeTokenFile } from "../electron/lib/token-file.js";
const dirs = [];
after(() => dirs.forEach((d) => fs.rmSync(d, { recursive: true, force: true })));
const TOKEN = `xenonet_actor_${"0a".repeat(32)}`;

// A made-up Windows: whoami says the account's SID, and icacls answers with
// the ACL a test gives, written as icacls /save writes it (UTF-16, after a
// line with the file's name). Setting an ACL records what the file held.
const SID = "S-1-5-21-1111111111-2222222222-3333333333-1001";
function windows({ acl = `D:AI(A;ID;FA;;;SY)(A;ID;FA;;;BA)(A;ID;FA;;;${SID})`, bom = true, fail = false } = {}) {
  const calls = [];
  const run = (file, args) => {
    calls.push([file, ...args]);
    if (fail) throw new Error(`${file}: not found`);
    if (file === "whoami") return `"nova-pc\\nova","${SID}"\r\n`;
    if (file === "icacls" && args[1] === "/save") {
      fs.writeFileSync(args[2], Buffer.concat([bom ? Buffer.from([0xff, 0xfe]) : Buffer.alloc(0), Buffer.from(`${path.basename(args[0])}\r\n${acl}\r\n`, "utf16le")]));
      return "";
    }
    if (file === "icacls") return void calls.at(-1).push(fs.readFileSync(args[0], "utf8"));
    throw new Error(`no ${file} on this made-up machine`);
  };
  return { run, calls, platform: "win32" };
}
const tempDir = () => {
  const dir = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-win-token-")));
  dirs.push(dir);
  return dir;
};

test("on Windows, a token file's ACL decides who can read it: yours, SYSTEM and the Administrators only, as OpenSSH's check for keys has it", () => {
  const file = path.join(tempDir(), "wren.token");
  fs.writeFileSync(file, `${TOKEN}\n`, { mode: 0o644 });
  assert.equal(readTokenFile(file, windows()), TOKEN, "a file in your user folder, as it inherits");
  assert.equal(readTokenFile(file, windows({ bom: false, acl: `D:P(A;;FA;;;${SID})` })), TOKEN, "yours alone");
  assert.equal(readTokenFile(file, windows({ acl: "D:P(A;;FA;;;LA)" })), TOKEN, "the built-in local Administrator, including a renamed account");
  assert.equal(readTokenFile(file, windows({ acl: `D:AI(A;OICIIO;GA;;;CO)(A;ID;FA;;;${SID})` })), TOKEN, "an entry only for what's made inside doesn't count");
  assert.equal(readTokenFile(file, windows({ acl: `D:AI(A;ID;FA;;;${SID})(D;;FA;;;WD)` })), TOKEN, "a deny entry lets nobody in");
  for (const [acl, who] of [
    [`D:AI(A;ID;FA;;;${SID})(A;ID;0x1200a9;;;BU)`, "Users, as in C:\\Users\\Public"],
    ["D:AI(A;ID;FA;;;SY)(A;ID;FR;;;WD)", "Everyone"],
    ["D:P(A;;FR;;;LG)", "the local Guest account"],
    [`D:AI(A;ID;FA;;;${SID})(A;ID;FR;;;AU)`, "Authenticated Users"],
    [`D:AI(A;ID;FA;;;${SID})(A;ID;FR;;;S-1-5-21-1111111111-2222222222-3333333333-1002)`, "another account"],
    ["D:NO_ACCESS_CONTROL", "no ACL at all"],
  ])
    assert.throws(() => readTokenFile(file, windows({ acl })), /other users can read .* icacls/, who);
  assert.throws(() => readTokenFile(file, windows({ fail: true })), /couldn't tell who else can read/, "one it can't ask about isn't trusted");
  assert.throws(() => readTokenFile(file, windows({ acl: "nothing like an ACL" })), /couldn't tell who else can read/);
  assert.deepEqual(fs.readdirSync(os.tmpdir()).filter((f) => f.startsWith(`xenonet-acl-${process.pid}-`)), [], "icacls's answer is taken away");
});

test("on Windows, a token file is written new and empty, made yours alone, and only then given the token", () => {
  const dir = tempDir();
  const w = windows();
  const old = path.join(dir, "config", "wren", "xenonet.token");
  fs.mkdirSync(path.dirname(old), { recursive: true });
  fs.writeFileSync(old, "an older file, with entries of its own\n");
  const file = writeTokenFile(old, TOKEN, { platform: "win32", run: w.run, force: true });
  assert.deepEqual(w.calls.find((c) => c[0] === "icacls"), ["icacls", file, "/inheritance:r", "/grant:r", `*${SID}:F`, "/q", ""], "nothing inherited, your account alone, and nothing in it yet");
  assert.equal(fs.readFileSync(file, "utf8"), `${TOKEN}\n`);
  const denied = (cmd, args) => (cmd === "whoami" ? w.run(cmd, args) : assert.fail("icacls: Access is denied."));
  const other = path.join(dir, "other.token");
  assert.throws(() => writeTokenFile(other, TOKEN, { platform: "win32", run: denied }), /couldn't make .* yours alone, so it didn't write the token there/);
  assert.equal(fs.existsSync(other), false, "nor is an empty one left behind");
});
