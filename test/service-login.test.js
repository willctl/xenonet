// Starting the service at login, as Desktop asks for it (electron/background.js)
// and the page says it (src/background.js), with a made-up xenonet command:
// nothing here registers anything with launchd or systemd.
import { test } from "node:test";
import assert from "node:assert/strict";
import { createBackground, said } from "../electron/background.js";
import { backgroundWords } from "../src/background.js";

const login = (over = {}) => ({ supported: true, supervisor: "launchd", file: "/Users/will/Library/LaunchAgents/dev.xenonet.service.plist", registered: false, port: null, loaded: false, state: null, pid: null, lastExit: null, ...over });

// A xenonet command that answers from `answers`, by its arguments, and
// keeps what it was asked.
function fake(answers) {
  const calls = [];
  const run = async (file, args, { env }) => {
    calls.push({ file, args, env });
    const a = answers[args.join(" ")];
    if (!a) return { code: 2, out: "", err: `xenonet: no answer for ${args.join(" ")}` };
    return typeof a === "function" ? a() : a;
  };
  return { calls, run };
}

function prefsOf(data = {}) {
  return { data, get: (k) => data[k], set: (k, v) => (data[k] = v) };
}

const make = (answers, over = {}) => {
  const f = fake(answers);
  const prefs = over.prefs ?? prefsOf();
  const b = createBackground({ bin: () => "/x/xenonet", env: () => ({ XENONET_PORT: "47840" }), live: true, port: 47840, home: "/Users/will", prefs, run: f.run, ...over });
  return { b, calls: f.calls, prefs };
};

const ok = (value) => ({ code: 0, out: JSON.stringify(value), err: "" });

test("status: what the command says, with the file under ~ and the offer due once", async () => {
  const st = { running: true, supervised: false, pid: 10, login: login() };
  const { b, calls, prefs } = make({ "service status --json": ok(st) });
  const s = await b.status();
  assert.equal(calls[0].file, "/x/xenonet");
  assert.deepEqual(calls[0].env, { XENONET_PORT: "47840" });
  assert.deepEqual(
    { supported: s.supported, live: s.live, registered: s.registered, running: s.running, supervised: s.supervised, supervisor: s.supervisor, file: s.file, offer: s.offer },
    { supported: true, live: true, registered: false, running: true, supervised: false, supervisor: "launchd", file: "~/Library/LaunchAgents/dev.xenonet.service.plist", offer: true },
  );
  prefs.set("askedBackground", true);
  assert.equal((await b.status()).offer, false, "asked once");
});

test("status: no service running still says how it starts, and Windows says it can't", async () => {
  const { b } = make({ "service status --json": { code: 1, out: JSON.stringify({ running: false, error: "ECONNREFUSED", login: login({ registered: true, port: "47840", loaded: true }) }), err: "" } });
  const s = await b.status();
  assert.equal(s.running, false);
  assert.equal(s.registered, true);
  assert.equal(s.offer, false, "already on");
  const reason = "Starting the service at login isn't available on Windows yet. Desktop starts the service when it opens.";
  const win = make({ "service status --json": ok({ running: true, login: { supported: false, supervisor: null, registered: false, reason } }) }, { platform: "win32" }).b;
  const w = await win.status();
  assert.deepEqual([w.supported, w.offer, w.reason], [false, false, reason]);
  assert.equal(backgroundWords(w), reason);
  // Tools from before this setting.
  const old = make({ "service status --json": ok({ running: true, pid: 3, version: "0.2.0", terminals: [] }) }).b;
  assert.match((await old.status()).reason, /older than Desktop/);
  // A command that couldn't run at all.
  const broken = make({ "service status --json": { code: 1, out: "", err: "spawn ENOENT" } }).b;
  const s2 = await broken.status();
  assert.deepEqual([s2.supported, s2.offer, s2.reason], [false, false, "spawn ENOENT"]);
});

test("set: install and uninstall through the command, which answers the offer too", async () => {
  let stopped = 0;
  const st = { running: true, login: login({ registered: true, port: "47840" }) };
  const { b, calls, prefs } = make(
    {
      "service install --json": ok({ status: "installed", supervisor: "launchd", takeover: "now" }),
      "service uninstall --json": ok({ status: "removed", stopped: true }),
      "service uninstall --json --force": ok({ status: "removed", stopped: false }),
      "service status --json": ok(st),
    },
    { afterStop: async () => void stopped++ },
  );
  const on = await b.act("set", { on: true });
  assert.equal(prefs.get("askedBackground"), true);
  assert.deepEqual(calls[0].args, ["service", "install", "--json"]);
  assert.equal(on.result.takeover, "now");
  assert.equal(on.registered, true);
  await b.act("set", { on: false });
  assert.equal(stopped, 1, "Desktop starts its own again once the supervisor's stopped");
  await b.act("set", { on: false, force: true });
  assert.equal(stopped, 1);
  assert.deepEqual(calls.at(-2).args, ["service", "uninstall", "--json", "--force"]);
});

test("set: the command's refusal is what the page shows, and XenoNet Dev doesn't register", async () => {
  const refusal = "this is yours to do, and it came from under a Claude Code process; run it in your own terminal or in XenoNet";
  const { b } = make({ "service install --json": { code: 1, out: "", err: `xenonet: ${refusal}\n` } });
  await assert.rejects(b.set(true), (err) => err.message === refusal);
  const dev = make({}, { live: false });
  await assert.rejects(dev.b.set(true), /XenoNet Dev starts and stops its own service/);
  const st = await dev.b.status();
  assert.deepEqual([st.live, st.offer, st.registered], [false, false, false]);
  assert.equal(dev.calls.length, 0, "XenoNet Dev never asks the command");
  assert.equal(said({ code: 3, out: "", err: "" }), "xenonet exited with code 3");
});

test("startRegistered: the supervisor starts a service registered for this port, and only that", async () => {
  const mine = make({ "service status --json": ok({ running: false, login: login({ registered: true, port: "47840" }) }), "service start": ok({ status: "started", supervisor: "launchd" }) });
  assert.equal(await mine.b.startRegistered(), "started");
  assert.deepEqual(mine.calls.at(-1).args, ["service", "start"]);
  const other = make({ "service status --json": ok({ running: false, login: login({ registered: true, port: "47842" }) }) });
  assert.equal(await other.b.startRegistered(), null);
  const none = make({ "service status --json": ok({ running: false, login: login() }) });
  assert.equal(await none.b.startRegistered(), null);
  const failing = make({ "service status --json": ok({ running: false, login: login({ registered: true, port: "47840" }) }), "service start": { code: 1, out: "", err: "xenonet: launchctl kickstart failed: no" } });
  await assert.rejects(failing.b.startRegistered(), /launchctl kickstart failed/);
});

test("remove: the uninstaller takes the registration out, terminals or not", async () => {
  const { b, calls } = make({ "service uninstall --json --force": ok({ status: "removed", stopped: true }) });
  assert.equal(await b.remove(), true);
  assert.deepEqual(calls[0].args, ["service", "uninstall", "--json", "--force"]);
  const none = make({ "service uninstall --json --force": ok({ status: "absent" }) });
  assert.equal(await none.b.remove(), false);
});

test("the panel's words follow where the setting stands", () => {
  const base = { live: true, supported: true, supervisor: "launchd" };
  assert.match(backgroundWords({ ...base, registered: false }), /^Off\. Desktop starts the service when it opens/);
  assert.match(backgroundWords({ ...base, registered: true, running: true, supervised: true }), /^On\. The service is running under launchd/);
  assert.match(backgroundWords({ ...base, registered: true, running: true, supervised: false }), /takes over when it stops/);
  assert.match(backgroundWords({ ...base, registered: true, running: false }), /^On\. launchd starts the service at login/);
  assert.match(backgroundWords({ ...base, live: false }), /XenoNet Dev starts and stops its own service/);
  assert.match(backgroundWords({ browser: true }), /xenonet service install/);
  assert.match(backgroundWords({ ...base, error: "launchctl couldn't start" }), /couldn't ask launchd/);
});
