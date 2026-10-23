// The doctor's look for actors' tokens left in repos (findTokenFiles and
// createTokenScan in core/actors/token-file.js), against real git in
// made-up repos: what's tracked, what's untracked, and what's ignored (a
// .env) outside the folders of dependencies, only the tokens XenoNet
// issued, and a look kept for a day unless you ask. And on a made-up
// Windows, a token file's ACL, read and written. The made-up tokens are
// built here, so no file of XenoNet's holds one.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { findTokenFiles, createTokenScan, TOKEN_SCAN_MS, readTokenFile, writeTokenFile } from "../core/actors/token-file.js";
import { hashToken } from "../core/actors/tokens.js";
import { machine } from "../core/doctor/index.js";

const dirs = [];
after(() => dirs.forEach((d) => fs.rmSync(d, { recursive: true, force: true })));
const TOKEN = `xenonet_actor_${"0a".repeat(32)}`;
const write = (file, text) => {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, text);
};

test("findTokenFiles: the files in repos that hold a token, tracked, untracked or ignored, but not in the folders of dependencies; a folder that isn't a repo isn't counted", async () => {
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-token-files-")));
  dirs.push(root);
  const repo = path.join(root, "atlas");
  fs.mkdirSync(repo, { recursive: true });
  execFileSync("git", ["init", "-q"], { cwd: repo });
  write(path.join(repo, ".gitignore"), ".env\nignored.token\nnode_modules/\nlogs/\n");
  write(path.join(repo, "config", "wren.token"), `${TOKEN}\n`);
  write(path.join(repo, ".env"), `WREN_TOKEN=${TOKEN}\n`);
  write(path.join(repo, "ignored.token"), `${TOKEN}\n`);
  write(path.join(repo, "notes.md"), "xenonet_actor_ is the prefix, and that's all\n");
  // Dependencies' caches aren't yours, but what's tracked there is looked in.
  write(path.join(repo, "node_modules", "pkg", "cache.json"), `{"t":"${TOKEN}"}\n`);
  write(path.join(repo, "logs", "kept.log"), `${TOKEN}\n`);
  // A name that holds a token itself.
  write(path.join(repo, "scratch", `${TOKEN}.txt`), `${TOKEN}\n`);
  execFileSync("git", ["add", "config/wren.token"], { cwd: repo });
  execFileSync("git", ["add", "-f", "logs/kept.log"], { cwd: repo });
  const clean = path.join(root, "ledger");
  fs.mkdirSync(clean);
  execFileSync("git", ["init", "-q"], { cwd: clean });
  write(path.join(clean, "README.md"), "nothing here\n");
  const plain = path.join(root, "plain");
  fs.mkdirSync(plain);
  const found = await findTokenFiles([repo, clean, plain], { run: machine().run, issued: [hashToken(TOKEN)] });
  const files = [".env", "config/wren.token", "ignored.token", "logs/kept.log", "scratch/xenonet_actor_….txt"].map((f) => path.join(repo, f));
  assert.deepEqual(found, { checked: 2, files });
  assert.ok(!JSON.stringify(found).includes(TOKEN), "never a token, even in a name");
  // Without XenoNet's own tokens to hold them to, every token counts, and
  // the look says it couldn't check them.
  assert.deepEqual(await findTokenFiles([repo, clean, plain], { run: machine().run }), { checked: 2, files, unchecked: true });
  // Nothing past the time it's given is started, and a repo's two looks
  // share its time.
  let t = 0;
  const slow = (ms) => async (...args) => ((t += ms), machine().run(...args));
  const late = await findTokenFiles([repo, clean], { run: slow(1000), budgetMs: 1500, together: 1, now: () => t });
  assert.equal(late.checked, 1);
  t = 0;
  const spent = await findTokenFiles([repo], { run: slow(3000), together: 1, now: () => t });
  assert.equal(spent.checked, 0, "its first look took all of its time");
});

test("findTokenFiles: a made-up token in a test fixture or a fuzz seed isn't one XenoNet issued, so only the file with an issued one counts", async () => {
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-token-issued-")));
  dirs.push(root);
  const repo = path.join(root, "xenonet");
  fs.mkdirSync(repo, { recursive: true });
  execFileSync("git", ["init", "-q"], { cwd: repo });
  const madeUp = `xenonet_gateway_${"a".repeat(64)}`;
  const issued = `xenonet_gateway_${"0123456789abcdef".repeat(4)}`;
  write(path.join(repo, ".gitignore"), ".env\n");
  write(path.join(repo, "tests", "fixtures", "gateway.json"), `{"token":"${madeUp}"}\n`);
  write(path.join(repo, "fuzz", "seeds", "http", "gateway-0"), `Authorization: Bearer ${madeUp}\r\n`);
  write(path.join(repo, ".env"), `MADE_UP=${madeUp}\nGATEWAY=${issued}\n`);
  execFileSync("git", ["add", "tests", "fuzz"], { cwd: repo });
  const look = (hashes) => findTokenFiles([repo], { run: machine().run, issued: hashes });
  assert.deepEqual(await look(new Set([hashToken(issued), hashToken(TOKEN)])), { checked: 1, files: [path.join(repo, ".env")] });
  assert.deepEqual(await look([]), { checked: 1, files: [] }, "none issued, none found");
  assert.deepEqual((await findTokenFiles([repo], { run: machine().run })).files.length, 3, "without the hashes, the fixture and the seed count too");
});

test("createTokenScan: a look kept for a day, unless you run the doctor yourself; never two at once", async () => {
  let t = 0;
  let looks = 0;
  let repos = ["/Users/nova/Projects/atlas-api"];
  // A made-up git: each repo answered with nothing found.
  const run = async () => (looks++, { code: 1, stdout: "", timedOut: false });
  const scan = createTokenScan({ repos: () => repos, run, issued: () => [], now: () => t });
  assert.equal(scan.last(), null);
  const first = await scan.look();
  assert.deepEqual(first, { at: 0, repos: 1, checked: 1, files: [] });
  const each = looks;
  t += TOKEN_SCAN_MS - 1;
  assert.equal(await scan.look(), first, "the schedule's runs within a day take the last look");
  assert.equal(looks, each);
  repos = [...repos, "/Users/nova/Projects/ledger"];
  assert.deepEqual(await scan.look({ fresh: true }), { at: t, repos: 2, checked: 2, files: [] }, "a doctor you run looks again");
  t += TOKEN_SCAN_MS;
  const [a, b] = await Promise.all([scan.look(), scan.look({ fresh: true })]);
  assert.equal(a, b, "one look at a time");
  assert.equal(TOKEN_SCAN_MS, 24 * 3600e3);
  // A store that can't be read leaves the look unchecked.
  const blind = createTokenScan({ repos: () => repos, run, issued: () => { throw new Error("database is locked"); }, now: () => t });
  assert.deepEqual(await blind.look(), { at: t, repos: 2, checked: 2, files: [], unchecked: true });
});

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
