// Reading the output of ps and netstat, and telling which programs are the
// agent apps (core/procs.js). The live lookups are macOS-only and talk to
// the real system, so only a couple of them run here, guarded by platform;
// the parsing is pure and runs everywhere.
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import net from "node:net";
import { parsePs, parseNetstat, parseCwd, parseCwds, parseCommands, parseTexts, agentOf, productOf, wantsArgs, createProcs, selfTest, parseTable, parseNetstatWindows, parseProcNetTcp, PROCESSES, TCP_PEER, COMMAND_LINE } from "../core/procs.js";
import { fakeProcesses } from "./harness/procs.js";

test("agentOf: the agent apps by their program path, and Node by its arguments", () => {
  assert.equal(agentOf("/Users/me/.local/bin/claude"), "claude");
  assert.equal(agentOf("/Users/me/.local/share/claude/versions/2.1.288"), "claude");
  assert.equal(agentOf("/Applications/Claude.app/Contents/MacOS/claude"), "claude");
  assert.equal(agentOf("/opt/homebrew/bin/codex"), "codex");
  assert.equal(agentOf("/path/codex-aarch64-apple-darwin"), "codex");
  // Node and Bun are told apart only by what they run.
  assert.equal(agentOf("/usr/local/bin/node"), null);
  assert.equal(agentOf("/usr/local/bin/node", "node /x/@anthropic-ai/claude-code/cli.js"), "claude");
  assert.equal(agentOf("/usr/local/bin/bun", "bun /x/@openai/codex/bin/codex.js"), "codex");
  // A program that only looks like one isn't.
  assert.equal(agentOf("/tmp/claude-decoy/run"), null);
  assert.equal(agentOf("/usr/bin/codexterm"), null);
  assert.equal(agentOf(""), null);
  assert.equal(agentOf(null), null);
});

// Made-up paths throughout: a home folder of nova's, and the apps where
// their makers put them.
const HERMES_PY = "/Users/nova/.hermes/tools/python-3.14.7/bin/python3.14";
const HERMES_APP = "/Users/nova/.hermes/hermes-agent/apps/desktop/release/mac-arm64/Hermes.app";
const GROK = "/Applications/Grok Bot.app";

test("productOf: Hermes's agent hosts and terminal UI by path and arguments, and actor apps by their bundle", () => {
  // Hermes's hosts: its own Pythons, running its CLI, its TUI's backend or its agent.
  assert.deepEqual(productOf(HERMES_PY, `${HERMES_PY} -I -c import hermes_cli.main; hermes_cli.main.main() serve`), { product: "hermes-agent", role: "host" });
  assert.deepEqual(productOf(HERMES_PY, `${HERMES_PY} -m tui_gateway.entry`), { product: "hermes-agent", role: "host" });
  assert.deepEqual(productOf("/Users/nova/.hermes/hermes-agent/venv/bin/python3", "python3 run_agent.py"), { product: "hermes-agent", role: "host" });
  assert.deepEqual(productOf(`${HERMES_APP}/Contents/Resources/python/bin/python3`, "python3 -m hermes_cli.main serve --port 0"), { product: "hermes-agent", role: "host" }, "a host inside Hermes.app is a host, not the app");
  assert.deepEqual(productOf("/usr/local/bin/node", "node /Users/nova/.hermes/hermes-agent/ui-tui/dist/entry.js"), { product: "hermes-agent", role: "tui" });
  // Not Hermes: a Python elsewhere, or one of Hermes's running something else.
  assert.equal(productOf("/opt/homebrew/bin/python3", "python3 -m hermes_cli.main"), null, "a Python outside Hermes's folders");
  assert.equal(productOf(HERMES_PY, `${HERMES_PY} -m pip install requests`), null);
  assert.equal(productOf("/usr/local/bin/node", "node server.js"), null);
  // The apps whose own processes could relay for a cloud, helpers included.
  assert.deepEqual(productOf(`${GROK}/Contents/MacOS/Grok Bot`), { product: "grok-bot", role: "app" });
  assert.deepEqual(productOf(`${GROK}/Contents/Frameworks/Grok Bot Helper (Renderer).app/Contents/MacOS/Grok Bot Helper (Renderer)`), { product: "grok-bot", role: "app" });
  assert.deepEqual(productOf(`${HERMES_APP}/Contents/MacOS/Hermes`), { product: "hermes-agent", role: "app" });
  assert.deepEqual(productOf("/Applications/Muse.app/Contents/MacOS/Muse"), { product: "muse", role: "app" });
  assert.deepEqual(productOf("/Applications/ChatGPT.app/Contents/MacOS/ChatGPT"), { product: "dots", role: "app" });
  for (const other of ["/Applications/XenoNet.app/Contents/MacOS/XenoNet", "/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal", "/bin/zsh", "/tmp/Grok Bot", "", null]) assert.equal(productOf(other), null, String(other));
  // Whose arguments are worth a look.
  assert.deepEqual([wantsArgs("/usr/local/bin/node"), wantsArgs("/x/bun"), wantsArgs(HERMES_PY), wantsArgs("/opt/homebrew/bin/python3"), wantsArgs("/bin/zsh")], [true, true, true, false, false]);
});

test("parseCwd: the folder in lsof's answer, or null", () => {
  assert.equal(parseCwd("p4242\nfcwd\nn/Users/nova/Projects/docs site\n"), "/Users/nova/Projects/docs site");
  assert.equal(parseCwd("p4242\n"), null);
  assert.equal(parseCwd(""), null);
});

test("chain(): Hermes's host and Grok Bot's app among a command's ancestors, and where a process works", async () => {
  const m = fakeProcesses();
  const app = m.add({ ppid: 1, program: `${GROK}/Contents/MacOS/Grok Bot` });
  const helper = m.add({ ppid: app, program: `${GROK}/Contents/Frameworks/Grok Bot Helper.app/Contents/MacOS/Grok Bot Helper` });
  const shell = m.add({ ppid: helper, program: "/bin/zsh", cwd: "/Users/nova/Projects/pathfinder" });
  const git = m.add({ ppid: shell, program: "/usr/bin/git", args: ["status"], cwd: "/Users/nova/Projects/pathfinder" });
  const tui = m.add({ ppid: 1, program: "/usr/local/bin/node", args: ["/Users/nova/.hermes/hermes-agent/ui-tui/dist/entry.js"] });
  const host = m.add({ ppid: tui, program: HERMES_PY, args: ["-m", "tui_gateway.entry"] });
  const bash = m.add({ ppid: host, program: "/bin/bash", args: ["-c", "claude -p hi"] });
  const claude = m.add({ ppid: bash, program: "/Users/nova/.local/bin/claude" });
  const procs = createProcs({ platform: "darwin", exec: m.exec });
  assert.deepEqual((await procs.chain(git)).map((p) => [p.pid, p.agent, p.product?.product ?? null, p.product?.role ?? null]), [
    [git, null, null, null],
    [shell, null, null, null],
    [helper, null, "grok-bot", "app"],
    [app, null, "grok-bot", "app"],
  ]);
  assert.deepEqual((await procs.chain(claude)).map((p) => [p.agent, p.product?.role ?? null]), [["claude", null], [null, null], [null, "host"], [null, "tui"]], "an agent is an agent, whatever runs above it");
  assert.equal(await procs.cwd(git), "/Users/nova/Projects/pathfinder");
  assert.equal(await procs.cwd(tui), null, "a process whose folder lsof doesn't give");
  assert.equal(await procs.cwd(99999), null, "a process that's gone");
  assert.equal(await createProcs({ platform: "linux", exec: () => assert.fail("no lsof here") }).cwd(git), null);
});

test("parseCwds and parseCommands: several processes' folders and command lines, by pid", () => {
  assert.deepEqual([...parseCwds("p41\nfcwd\nn/Users/nova/Projects/docs site\np42\nfcwd\nn/\np43\n")], [[41, "/Users/nova/Projects/docs site"], [42, "/"]]);
  assert.deepEqual([...parseCwds("")], []);
  assert.deepEqual([...parseCommands("   41 /usr/local/bin/node /x/entry.js --flag\n 1042 /bin/zsh\nnot a line\n")], [[41, "/usr/local/bin/node /x/entry.js --flag"], [1042, "/bin/zsh"]]);
});

test("census(): every process with what it is; Node's and Hermes's Pythons' arguments read once, together, and never kept", async () => {
  const clock = { t: Date.parse("2026-10-05T12:00:00Z") };
  const m = fakeProcesses({ clock: { now: () => clock.t, setTimeout, clearTimeout } });
  const tui = m.add({ ppid: 1, program: "/usr/local/bin/node", args: ["/Users/nova/.hermes/hermes-agent/ui-tui/dist/entry.js"] });
  const host = m.add({ ppid: tui, program: HERMES_PY, args: ["-m", "tui_gateway.entry"] });
  const claude = m.add({ ppid: 1, program: "/usr/local/bin/node", args: ["/opt/lib/node_modules/@anthropic-ai/claude-code/cli.js"] });
  const native = m.add({ ppid: 1, program: "/Users/nova/.local/bin/claude" });
  const app = m.add({ ppid: 1, program: `${GROK}/Contents/MacOS/Grok Bot` });
  const shell = m.add({ ppid: app, program: "/bin/zsh" });
  const procs = createProcs({ platform: "darwin", exec: m.exec, now: () => clock.t });
  const what = (c, pid) => [c.of(c.table.get(pid)).agent, c.of(c.table.get(pid)).product?.role ?? null];
  const argsAsked = () => m.asked.filter((a) => a[0] === "ps" && a[2] === "pid=,command=");
  let c = await procs.census();
  assert.deepEqual([what(c, tui), what(c, host), what(c, claude), what(c, native), what(c, app), what(c, shell)], [[null, "tui"], [null, "host"], ["claude", null], ["claude", null], [null, "app"], [null, null]]);
  assert.equal(argsAsked().length, 1, "one ps for every new Node and Python process");
  // The table's own Node (this made-up machine's server) is one of them.
  assert.deepEqual(argsAsked()[0].at(-1).split(",").map(Number).sort(), [m.self.pid, tui, host, claude].sort(), "and only theirs");
  clock.t += 2000;
  c = await procs.census();
  assert.equal(argsAsked().length, 1, "nothing new, nothing asked");
  // A new one is asked about alone, and one gone before ps looked is asked
  // about again next time rather than taken for nothing.
  const late = m.add({ ppid: 1, program: "/usr/local/bin/node", args: ["/Users/nova/.hermes/hermes-agent/hermes_cli/tui_dist/entry.js"] });
  const exec = m.exec;
  let hide = true;
  const flaky = createProcs({ platform: "darwin", now: () => clock.t, exec: async (file, args) => (file === "ps" && args[1] === "pid=,command=" && hide ? "" : exec(file, args)) });
  assert.deepEqual(what(await flaky.census(), late), [null, null], "told by its program alone while its arguments can't be read");
  hide = false;
  clock.t += 2000;
  assert.deepEqual(what(await flaky.census(), late), [null, "tui"], "and asked again");
  clock.t += 2000;
  c = await procs.census();
  assert.deepEqual(what(c, late), [null, "tui"]);
  assert.deepEqual(argsAsked().at(-1).at(-1), String(late));
  assert.equal(JSON.stringify([...c.table.values()]).includes("tui_gateway"), false, "the table holds programs, not arguments");
  // Elsewhere, nothing to look at.
  const none = await createProcs({ platform: "linux", exec: () => assert.fail("no ps here") }).census();
  assert.deepEqual([none.table.size, none.of({ pid: 1 }).agent], [0, null]);
});

test("children(), describe() and cwds(): what an app runs, by pgrep, described and placed only for the new ones", async () => {
  const m = fakeProcesses();
  const app = m.add({ ppid: 1, program: `${GROK}/Contents/MacOS/Grok Bot` });
  const helper = m.add({ ppid: app, program: `${GROK}/Contents/Frameworks/Grok Bot Helper.app/Contents/MacOS/Grok Bot Helper` });
  const shell = m.add({ ppid: helper, program: "/bin/zsh", cwd: "/Users/nova/Projects/pathfinder" });
  const git = m.add({ ppid: shell, program: "/usr/bin/git", cwd: "/Users/nova/Projects/pathfinder" });
  const procs = createProcs({ platform: "darwin", exec: m.exec });
  assert.deepEqual((await procs.children([app])).sort(), [helper]);
  assert.deepEqual((await procs.children([app, helper, shell])).sort((a, b) => a - b), [helper, shell, git].sort((a, b) => a - b));
  assert.deepEqual(await procs.children([git]), [], "pgrep finds none: no children, not an error");
  assert.deepEqual(await procs.children([]), []);
  const described = await procs.describe([shell, git, 99999]);
  assert.deepEqual([...described.keys()].sort(), [shell, git].sort(), "one that's gone is left out");
  assert.deepEqual([described.get(git).ppid, described.get(git).program], [shell, "/usr/bin/git"]);
  assert.deepEqual([...(await procs.cwds([shell, git, helper]))], [[shell, "/Users/nova/Projects/pathfinder"], [git, "/Users/nova/Projects/pathfinder"]]);
  assert.equal(m.asked.filter((a) => a[0] === "ps" && a[1] === "-axo").length, 0, "none of it reads the whole table");
  const elsewhere = createProcs({ platform: "linux", exec: () => assert.fail("nothing to ask") });
  assert.deepEqual([await elsewhere.children([app]), (await elsewhere.describe([app])).size, (await elsewhere.cwds([app])).size], [[], 0, 0]);
});

test("parsePs: pid, parent, start time and program, with spaces in the path", () => {
  const text = [
    "    1     0 Thu Oct  2 09:00:00 2026 /sbin/launchd",
    "  510     1 Fri Oct  3 16:34:29 2026 /Users/me/Library/Application Support/Claude/claude.app/Contents/MacOS/claude",
    " 1620   510 Sat Oct  3 11:57:44 2026 /bin/zsh",
    "garbage line that should be skipped",
  ].join("\n");
  const t = parsePs(text);
  assert.equal(t.size, 3);
  assert.deepEqual(t.get(1620), { pid: 1620, ppid: 510, start: Date.UTC(2026, 9, 3, 11, 57, 44), program: "/bin/zsh" }, "ps runs with TZ=UTC, so lstart is read as UTC");
  assert.equal(t.get(510).program, "/Users/me/Library/Application Support/Claude/claude.app/Contents/MacOS/claude");
  assert.equal(t.get(510).ppid, 1);
});

test("parsePs: start times don't move with the zone XenoNet runs in, across the hour that repeats or a change of zone", async () => {
  // A process started at 01:30 local time on the night clocks went back
  // in Chicago, after they did (07:30 UTC, not 06:30), and one ps saw
  // while XenoNet kept the zone it started in. ps says both in UTC.
  const text = "  700     1 Sun Nov  1 07:30:00 2026 /usr/local/bin/node\n  701     1 Mon Oct  5 23:00:00 2026 /bin/zsh\n";
  const started = { 700: Date.parse("2026-11-01T07:30:00Z"), 701: Date.parse("2026-10-05T23:00:00Z") };
  for (const zone of ["America/Chicago", "Asia/Tokyo", "UTC"]) {
    const child = await new Promise((resolve, reject) =>
      execFile(process.execPath, ["--input-type=module", "-e", `import { parsePs } from ${JSON.stringify(new URL("../core/procs.js", import.meta.url).href)}; const t = parsePs(${JSON.stringify(text)}); console.log(JSON.stringify([t.get(700).start, t.get(701).start]));`], { env: { ...process.env, TZ: zone } }, (err, out) => (err ? reject(err) : resolve(out))),
    );
    assert.deepEqual(JSON.parse(child), [started[700], started[701]], zone);
  }
  assert.equal(parsePs("  702     1 Sun Foo  1 07:30:00 2026 /bin/sh\n").get(702).start, 0, "a month it doesn't know is no time");
});

test("the commands run with start times in English and UTC, and paths as UTF-8, whatever the locale", async (t) => {
  if (process.platform !== "darwin") return t.skip("ps's lstart and its escaping are macOS's");
  // XenoNet's own environment as the desktop app might leave it, in German:
  // ps would write "Mo.  5 Okt." and the table would come back empty.
  const saved = { LANG: process.env.LANG, LC_ALL: process.env.LC_ALL };
  Object.assign(process.env, { LANG: "de_DE.UTF-8", LC_ALL: "de_DE.UTF-8" });
  try {
    const me = (await createProcs().snapshot()).get(process.pid);
    assert.ok(me, "this process is in the table");
    assert.ok(Math.abs(me.start - (Date.now() - process.uptime() * 1000)) < 5000, `its start time is when it started (${new Date(me.start).toISOString()})`);
  } finally {
    for (const [k, v] of Object.entries(saved)) v === undefined ? delete process.env[k] : (process.env[k] = v);
  }
});

test("parseNetstat: the pid at the client end of the loopback connection", () => {
  // The server listens on 47842; a client on an ephemeral port 51000 holds
  // the other end. netstat names the process as program:pid, and the program
  // name can have spaces.
  const text = [
    "Active Internet connections",
    "Proto Recv-Q Send-Q  Local Address          Foreign Address        (state)    rhiwat  shiwat    pid   epid state    options           gencnt    flags   flags1 usscnt rtntype fltrs",
    "tcp4       0      0  127.0.0.1.47842        127.0.0.1.51000        ESTABLISHED   2970    111  406912  146988 XenoNet Helper:9100    00102 00000104 0000000003bca9ca 00000081 01000800      2      0 000000",
    "tcp4       0      0  127.0.0.1.51000        127.0.0.1.47842        ESTABLISHED    111   2970  406912  146988 node:4242            00102 00000104 0000000003bca9cb 00000081 01000800      2      0 000000",
  ].join("\n");
  assert.equal(parseNetstat(text, { localPort: 47842, remotePort: 51000 }), 4242);
  // Nothing matches when the pair is wrong.
  assert.equal(parseNetstat(text, { localPort: 47842, remotePort: 52000 }), null);
});

test("parseNetstat: a client that closed its side, or exited, is still the one at that end; an earlier connection's TIME_WAIT isn't", () => {
  // Made-up lines in netstat -anv's shape: the server on 47842, a client on
  // 51000. Once the client closes its side, XenoNet's end waits in
  // CLOSE_WAIT and the client's in FIN_WAIT_2, with the pid that held it
  // and no name once that process has exited.
  const row = (local, foreign, state, owner) => `tcp4       0      0  127.0.0.1.${local}        127.0.0.1.${foreign}        ${state}            0            0  408300  146988             ${owner}  00112 00000100 0000000004ad7525 00000081 04000900      2      0 000000`;
  const at = { localPort: 47842, remotePort: 51000 };
  const server = row(47842, 51000, "CLOSE_WAIT", "XenoNet Helper:9100");
  assert.equal(parseNetstat([server, row(51000, 47842, "FIN_WAIT_2", "curl:4300")].join("\n"), at), 4300, "half-closed by a live process");
  assert.equal(parseNetstat([server, row(51000, 47842, "FIN_WAIT_2", ":4301")].join("\n"), at), 4301, "closed by a process that has exited");
  assert.equal(parseNetstat(row(51000, 47842, "FIN_WAIT_1", "node:4302"), at), 4302);
  assert.equal(parseNetstat(row(51000, 47842, "TIME_WAIT", "XenoNet Helper:9100"), at), null, "TIME_WAIT on the client's side means both ends had closed: not the connection being answered");
  assert.equal(parseNetstat([row(51000, 47842, "FIN_WAIT_2", ":4301"), row(51000, 47842, "ESTABLISHED", "node:4303")].join("\n"), at), 4303, "an open one first");
  assert.equal(parseNetstat(server, at), null, "XenoNet's own end is never the client");
  assert.equal(parseNetstat(row(51000, 47842, "FIN_WAIT_2", "odd:name:4304"), at), 4304, "the pid is the last number, whatever the name");
});

test("chain(): a process newer than the shared table is read by itself, with its new parents, before it's taken for gone", async () => {
  const clock = { t: Date.parse("2026-10-05T12:00:00Z") };
  const m = fakeProcesses({ clock: { now: () => clock.t, setTimeout, clearTimeout } });
  const terminal = m.add({ ppid: 1, program: "/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal" });
  const shell = m.add({ ppid: terminal, program: "/bin/zsh" });
  const procs = createProcs({ platform: "darwin", exec: m.exec, now: () => clock.t });
  const tables = () => m.asked.filter((a) => a[0] === "ps" && a[1] === "-axo").length;
  const described = () => m.asked.filter((a) => a[0] === "ps" && a[1] === "-o" && a[2] === "pid=,ppid=,lstart=,comm=").map((a) => Number(a.at(-1)));
  assert.deepEqual((await procs.chain(shell)).map((p) => p.pid), [shell, terminal]);
  assert.deepEqual([tables(), described()], [1, []]);
  // A xenonet command started a moment after the table was read, under a
  // shell started then too.
  clock.t += 300;
  const sh = m.add({ ppid: shell, program: "/bin/sh" });
  const fresh = m.add({ ppid: sh, program: "/usr/bin/curl" });
  assert.deepEqual((await procs.chain(fresh)).map((p) => p.pid), [fresh, sh, shell, terminal], "found, with its parents");
  assert.deepEqual([tables(), described()], [1, [fresh, sh, shell]], "each read by itself, up to the first the table has as it is");
  // One that's gone is asked about once, and a pid seen before not at all.
  assert.deepEqual(await procs.chain(99999), []);
  assert.deepEqual(described().at(-1), 99999);
  const asked = described().length;
  assert.deepEqual(await procs.chain(99998, { again: false }), []);
  assert.equal(described().length, asked);
  // A parent whose pid was used again since the table was read isn't taken
  // for the one the table has: the zsh it had ended, and a bash took its pid.
  m.remove(shell);
  m.table.set(shell, { pid: shell, ppid: terminal, start: clock.t + 1000, program: "/bin/bash", args: [], cwd: null });
  const later = m.add({ ppid: shell, program: "/usr/bin/curl" });
  assert.deepEqual((await procs.chain(later)).map((p) => [p.pid, p.program]), [[later, "/usr/bin/curl"], [shell, "/bin/bash"], [terminal, "/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal"]]);
  assert.equal(tables(), 1, "and the whole table was read once");
});

test("parseTexts: each process's executable, the first program text lsof lists for it", () => {
  const text = ["p41", "fcwd", "n/Users/nova/Projects/docs site", "ftxt", "n/bin/zsh", "ftxt", "n/usr/lib/dyld", "p42", "ftxt", "n/Users/nova/Library/Application Support/Tools/python3.14", "ftxt", "n/usr/lib/libz.1.dylib", "p43", "fcwd", "n/"].join("\n");
  assert.deepEqual([...parseTexts(text)], [[41, "/bin/zsh"], [42, "/Users/nova/Library/Application Support/Tools/python3.14"]], "never a folder, and nothing for a process lsof gave no text for");
  assert.deepEqual([...parseTexts("")], []);
});

// A framework Python's real executable, which a plain python3 runs.
const FRAMEWORK_PY = "/opt/homebrew/Cellar/python@3.13/3.13.7/Frameworks/Python.framework/Versions/3.13/Resources/Python.app/Contents/MacOS/Python";

test("chain() with exe: what a process is goes by its executable's real path, not the argv[0] it gave itself", async () => {
  const m = fakeProcesses();
  const terminal = m.add({ ppid: 1, program: "/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal" });
  const shell = m.add({ ppid: terminal, program: "/bin/zsh" });
  // exec -a: a plain Python that names itself as Hermes's, running "hermes_cli",
  // and the xenonet command it runs.
  const posing = m.add({ ppid: shell, program: HERMES_PY, exe: FRAMEWORK_PY, args: ["-c", "import hermes_cli"] });
  const asks = m.add({ ppid: posing, program: "/usr/local/bin/node", args: ["/Users/nova/xenonet/cli/xenonet.js", "actor", "heartbeat"] });
  // Hermes's own Python, whatever it calls itself.
  const host = m.add({ ppid: shell, program: "python3", exe: HERMES_PY, args: ["-m", "tui_gateway.entry"] });
  // Claude Code under another name: an agent can't hide by renaming itself.
  const renamed = m.add({ ppid: shell, program: "/usr/local/bin/worker", exe: "/Users/nova/.local/share/claude/versions/2.1.300" });
  // One whose executable lsof can't read, naming itself a host.
  const unread = m.add({ ppid: shell, program: HERMES_PY, exe: null, args: ["-m", "tui_gateway.entry"] });
  const procs = createProcs({ platform: "darwin", exec: m.exec });
  const what = (c) => c.map((p) => [p.pid, p.exe, p.agent, p.product?.role ?? null]);
  assert.deepEqual(what(await procs.chain(asks, { exe: true })), [
    [asks, "/usr/local/bin/node", null, null],
    [posing, FRAMEWORK_PY, null, null],
    [shell, "/bin/zsh", null, null],
    [terminal, "/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal", null, null],
  ]);
  assert.deepEqual(what(await procs.chain(host, { exe: true }))[0], [host, HERMES_PY, null, "host"]);
  assert.deepEqual(what(await procs.chain(renamed, { exe: true }))[0], [renamed, "/Users/nova/.local/share/claude/versions/2.1.300", "claude", null]);
  assert.deepEqual(what(await procs.chain(unread, { exe: true }))[0], [unread, null, null, null], "argv[0] alone never makes an agent host");
  // Without exe, the checks that only ever refuse (yours, an agent's own)
  // go by the program ps names: a process naming itself a product only
  // refuses itself.
  assert.deepEqual((await procs.chain(posing)).map((p) => [p.exe, p.product?.role ?? null])[0], [undefined, "host"]);
  // Each process's executable is read once, in one lsof for a chain.
  const texts = () => m.asked.filter((a) => a[0] === "lsof" && a.includes("txt"));
  const before = texts().length;
  await procs.chain(asks, { exe: true });
  assert.equal(texts().length, before, "read before, not asked again");
  assert.equal(texts()[0][texts()[0].indexOf("-p") + 1].split(",").length, 4, "the whole chain in one lsof");
});

test("chain() with exe: lsof's list is read even when it says 1 for a process it couldn't read", async () => {
  const m = fakeProcesses();
  const shell = m.add({ ppid: 1, program: "/bin/zsh" });
  const node = m.add({ ppid: shell, program: "node", exe: "/usr/local/bin/node" });
  // As lsof does when another user's process is among those asked about.
  const exec = async (file, args) => {
    if (file !== "lsof" || !args.includes("txt")) return m.exec(file, args);
    throw Object.assign(new Error("lsof: some processes couldn't be read"), { code: 1, stdout: await m.exec(file, args) });
  };
  const chain = await createProcs({ platform: "darwin", exec }).chain(node, { exe: true });
  assert.deepEqual(chain.map((p) => p.exe), ["/usr/local/bin/node", "/bin/zsh"]);
});

test("Hermes.app is Hermes's only when its Info.plist says so: its id read once per bundle, for chains and the census", async () => {
  const m = fakeProcesses();
  const hermes = "/Users/nova/.hermes/hermes-agent/apps/desktop/release/mac-arm64/Hermes.app";
  const setup = "/Applications/Hermes.app";
  // Another maker's app with the same name, and one whose id can't be read.
  const mail = "/Users/nova/Applications/Hermes.app";
  const odd = "/Users/nova/Downloads/Hermes.app";
  m.bundle(hermes, "com.nousresearch.hermes");
  m.bundle(setup, "com.nousresearch.hermes.setup");
  m.bundle(mail, "com.example.hermes-mail");
  const app = m.add({ ppid: 1, program: `${hermes}/Contents/MacOS/Hermes` });
  const helper = m.add({ ppid: app, program: `${hermes}/Contents/Frameworks/Hermes Helper.app/Contents/MacOS/Hermes Helper` });
  const host = m.add({ ppid: app, program: `${hermes}/Contents/Resources/python/bin/python3`, args: ["-m", "hermes_cli.main", "serve"] });
  const installer = m.add({ ppid: 1, program: `${setup}/Contents/MacOS/Hermes` });
  const mailApp = m.add({ ppid: 1, program: `${mail}/Contents/MacOS/Hermes` });
  const mailPy = m.add({ ppid: mailApp, program: `${mail}/Contents/Resources/python/bin/python3`, args: ["-m", "hermes_cli.main"] });
  const oddApp = m.add({ ppid: 1, program: `${odd}/Contents/MacOS/Hermes` });
  const oddPy = m.add({ ppid: oddApp, program: `${odd}/Contents/Resources/python/bin/python3`, args: ["run_agent.py"] });
  const procs = createProcs({ platform: "darwin", exec: m.exec });
  const role = async (pid, exe = true) => (await procs.chain(pid, { exe }))[0].product?.role ?? null;
  assert.deepEqual([await role(app), await role(helper), await role(host), await role(installer)], ["app", "app", "host", "app"]);
  assert.deepEqual([await role(mailApp), await role(mailPy), await role(mailApp, false)], [null, null, null], "another maker's Hermes.app isn't Hermes's");
  assert.deepEqual([await role(oddApp), await role(oddPy)], ["app", null], "one whose id can't be read: its own processes still count against it, and its Python is no host");
  const c = await procs.census();
  const census = (pid) => c.of(c.table.get(pid)).product?.role ?? null;
  assert.deepEqual([app, helper, host, installer, mailApp, mailPy, oddApp, oddPy].map(census), ["app", "app", "host", "app", null, null, "app", null]);
  assert.deepEqual(m.asked.filter((a) => a[0] === "plutil").map((a) => a.slice(1, 6)), Array(4).fill(["-extract", "CFBundleIdentifier", "raw", "-o", "-"]), "only that key, once for each of the four bundles");
});

test("Muse.app is Meta's Muse only when its Info.plist says com.meta.endo: other Mac apps are called Muse", async () => {
  const m = fakeProcesses();
  const muse = "/Applications/Muse.app";
  const notes = "/Users/nova/Applications/Muse.app";
  m.bundle(muse, "com.meta.endo");
  m.bundle(notes, "com.example.muse-notes");
  const app = m.add({ ppid: 1, program: `${muse}/Contents/MacOS/Muse` });
  const crashpad = m.add({ ppid: 1, program: `${muse}/Contents/MacOS/EndoCrashpadHandler` });
  const updater = m.add({ ppid: app, program: `${muse}/Contents/Frameworks/Sparkle.framework/Versions/B/Updater.app/Contents/MacOS/Updater` });
  const other = m.add({ ppid: 1, program: `${notes}/Contents/MacOS/Muse` });
  const procs = createProcs({ platform: "darwin", exec: m.exec });
  const product = async (pid) => (await procs.chain(pid, { exe: true }))[0].product;
  assert.deepEqual(await product(app), { product: "muse", role: "app" });
  assert.deepEqual([await product(crashpad), await product(updater)], [{ product: "muse", role: "app" }, { product: "muse", role: "app" }], "its helpers, and the updater inside it");
  assert.equal(await product(other), null, "another maker's Muse.app isn't Meta's");
  const c = await procs.census();
  assert.deepEqual([app, crashpad, updater, other].map((pid) => c.of(c.table.get(pid)).product?.product ?? null), ["muse", "muse", "muse", null]);
});

test("Grok Bot.app is Grok Bot's only when its Info.plist says com.anysphere.sand: anyone can name a bundle that", async () => {
  const m = fakeProcesses();
  const grok = "/Applications/Grok Bot.app";
  const fake = "/Users/nova/Downloads/Grok Bot.app";
  m.bundle(grok, "com.anysphere.sand");
  m.bundle(fake, "com.example.not-grok");
  const app = m.add({ ppid: 1, program: `${grok}/Contents/MacOS/Grok Bot` });
  const helper = m.add({ ppid: app, program: `${grok}/Contents/Frameworks/Grok Bot Helper.app/Contents/MacOS/Grok Bot Helper` });
  const other = m.add({ ppid: 1, program: `${fake}/Contents/MacOS/Grok Bot` });
  const procs = createProcs({ platform: "darwin", exec: m.exec });
  const product = async (pid) => (await procs.chain(pid, { exe: true }))[0].product;
  assert.deepEqual([await product(app), await product(helper)], [{ product: "grok-bot", role: "app" }, { product: "grok-bot", role: "app" }]);
  assert.equal(await product(other), null, "another bundle named Grok Bot.app isn't Grok Bot's");
  const c = await procs.census();
  assert.deepEqual([app, helper, other].map((pid) => c.of(c.table.get(pid)).product?.product ?? null), ["grok-bot", "grok-bot", null]);
});

test("the census reads a few new Hermes.app bundles at a time, since programs name themselves", async () => {
  const m = fakeProcesses();
  for (let i = 0; i < 40; i++) m.add({ ppid: 1, program: `/Users/nova/fake-${i}/Hermes.app/Contents/MacOS/Hermes` });
  const procs = createProcs({ platform: "darwin", exec: m.exec });
  const reads = () => m.asked.filter((a) => a[0] === "plutil").length;
  await procs.census();
  assert.equal(reads(), 16);
  await procs.census();
  assert.equal(reads(), 32, "then the next ones, and never one read before");
});

test("peer(): only a process found is kept on the socket, so a lookup that failed is tried again", async () => {
  let netstat = "";
  const asked = [];
  const procs = createProcs({ platform: "darwin", exec: async (file) => (asked.push(file), file === "netstat" ? netstat : "") });
  const socket = { localPort: 47842, remotePort: 51000 };
  assert.equal(await procs.peer(socket), null);
  netstat = "tcp4       0      0  127.0.0.1.51000        127.0.0.1.47842        ESTABLISHED            0            0  408300  146988             node:4242  00102 00000100 0000000004ad7525 00000081 04000900      2      0 000000";
  assert.equal(await procs.peer(socket), 4242, "asked again");
  netstat = "";
  assert.equal(await procs.peer(socket), 4242, "then kept");
  assert.equal(asked.filter((f) => f === "netstat").length, 2);
});

test("createProcs on another platform supports nothing and looks nowhere", async () => {
  const procs = createProcs({ platform: "freebsd", exec: async () => assert.fail("should not run commands") });
  assert.deepEqual([procs.supported, procs.chains], [false, false]);
  assert.equal(await procs.peer({ remotePort: 51000, localPort: 47842 }), null);
  assert.equal(await procs.signed({ agent: "claude", pid: 1, start: 0, program: "/x/claude" }), false);
});

// A made-up Linux or Windows machine: what each call answers, by its file and
// arguments; `asked` keeps every call.
function nativeMachine(answers) {
  const asked = [];
  const exec = async (file, args) => {
    const key = [file, ...args].join(" ");
    asked.push(key);
    if (!(key in answers)) throw new Error(`no ${key} here`);
    return answers[key];
  };
  return { asked, exec, answers };
}

test("on Windows the processes are read natively, the connection's process from netstat -ano, and a reused parent pid isn't a parent", async () => {
  // Claude Code's claude.exe runs cmd, which runs the xenonet command; Node
  // runs the npm Claude Code, known by its command line. 30's parent pid 9
  // was a process that's gone, and 9 now belongs to one that started later.
  // Claude desktop's own claude.exe isn't Claude Code.
  const table = [
    "4\t0\t0\tSystem\t",
    "5\t4\t1000\tC:\\Windows\\explorer.exe\tC:\\Windows\\explorer.exe",
    "20\t5\t2000\tC:\\Users\\me\\.local\\bin\\claude.exe\tC:\\Users\\me\\.local\\bin\\claude.exe",
    "21\t20\t3000\tC:\\Windows\\System32\\cmd.exe\tC:\\Windows\\System32\\cmd.exe",
    "22\t21\t4000\tC:\\Users\\me\\AppData\\Local\\XenoNet\\runtime\\launcher\\xenonet.exe\tC:\\Users\\me\\AppData\\Local\\XenoNet\\runtime\\launcher\\xenonet.exe",
    "23\t5\t2500\tC:\\Program Files\\nodejs\\node.exe\tC:\\Program Files\\nodejs\\node.exe",
    "9\t5\t9000\tC:\\Windows\\notepad.exe\tC:\\Windows\\notepad.exe",
    "30\t9\t5000\tC:\\Windows\\System32\\cmd.exe\t",
    "31\t1\t5000\tC:\\Users\\me\\AppData\\Local\\AnthropicClaude\\app-1.0.0\\claude.exe\tC:\\Users\\me\\AppData\\Local\\AnthropicClaude\\app-1.0.0\\claude.exe",
  ].join("\r\n");
  // German Windows: the state words are the system's language.
  const netstat = "\r\nAktive Verbindungen\r\n\r\n  Proto  Lokale Adresse         Remoteadresse          Status           PID\r\n  TCP    127.0.0.1:47840        0.0.0.0:0              ABHÖREN          700\r\n  TCP    127.0.0.1:47840        127.0.0.1:51000        HERGESTELLT      700\r\n  TCP    127.0.0.1:51000        127.0.0.1:47840        HERGESTELLT      22\r\n  TCP    127.0.0.1:51001        127.0.0.1:47840        TIME_WAIT        0\r\n";
  const m = nativeMachine({ [PROCESSES]: table, "netstat -ano -p TCP": netstat, [`${COMMAND_LINE} 23`]: '"C:\\Program Files\\nodejs\\node.exe" C:\\Users\\me\\AppData\\Roaming\\npm\\node_modules\\@anthropic-ai\\claude-code\\cli.js' });
  const procs = createProcs({ platform: "win32", exec: m.exec, now: () => 1e12 });
  assert.deepEqual([procs.supported, procs.chains], [false, true]);
  const socket = { localPort: 47840, remotePort: 51000 };
  assert.equal(await procs.peer(socket), 22, "the client's end, whatever the state is called in this language");
  assert.equal(await procs.peer({ localPort: 47840, remotePort: 51001 }), null, "a closed pair's pid is 0");
  const chain = await procs.chain(22, { exe: true });
  assert.deepEqual(chain.map((p) => [p.pid, p.agent]), [[22, null], [21, null], [20, "claude"], [5, null], [4, null]]);
  assert.equal(chain[0].exe, "C:\\Users\\me\\AppData\\Local\\XenoNet\\runtime\\launcher\\xenonet.exe");
  assert.equal((await procs.chain(23))[0].agent, "claude", "node.exe running Claude Code's cli.js");
  assert.deepEqual((await procs.chain(30, { exe: true })).map((p) => [p.pid, p.exe]), [[30, null]], "9 started after 30, so it isn't 30's parent");
  assert.equal((await procs.chain(31))[0].agent, null, "Claude desktop's own claude.exe isn't Claude Code");
  assert.ok(!m.asked.some((a) => a.startsWith("ps ") || a.startsWith("lsof ")), m.asked.join("\n"));
  assert.equal(parseNetstatWindows(netstat, socket), 22);
});

test("on Linux the processes and a connection's process are read from /proc, and a newcomer by reading the table again", async () => {
  const first = "1\t0\t100\t/sbin/init\t/usr/lib/systemd/systemd\n50\t1\t200\t/bin/bash\t/usr/bin/bash\n";
  const m = nativeMachine({ [PROCESSES]: first, [`${TCP_PEER} 47840 51000`]: "60\n" });
  let clock = 1e12;
  const procs = createProcs({ platform: "linux", exec: m.exec, now: () => clock });
  assert.deepEqual([procs.supported, procs.chains], [false, true]);
  assert.equal(await procs.peer({ localPort: 47840, remotePort: 51000 }), 60);
  assert.equal((await procs.chain(50)).length, 1, "up to init, as up to launchd");
  m.answers[PROCESSES] = `${first}60\t50\t300\tclaude\t/home/me/.local/share/claude/versions/2.1.0\n`;
  const chain = await procs.chain(60, { exe: true });
  assert.deepEqual(chain.map((p) => [p.pid, p.agent]), [[60, "claude"], [50, null]]);
  assert.equal(chain[0].exe, "/home/me/.local/share/claude/versions/2.1.0");
  // What /proc/net/tcp and a table line say.
  const tcp = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n   0: 0100007F:BAB0 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 11111 1 0000000000000000 100 0 0 10 0\n   1: 0100007F:C738 0100007F:BAB0 01 00000000:00000000 00:00000000 00000000  1000        0 22222 1 0000000000000000 20 4 30 10 -1\n   2: 0100007F:C739 0100007F:BAB0 06 00000000:00000000 03:00000000 00000000     0        0 0 3 0000000000000000\n";
  assert.equal(parseProcNetTcp(tcp, { localPort: 47792, remotePort: 51000 }), "22222");
  assert.equal(parseProcNetTcp(tcp, { localPort: 47792, remotePort: 51001 }), null, "a socket that's closing has no inode");
  const { table, exes } = parseTable("7\t1\t1.5e12\tpython3 -c x\t\n8\tx\tnope\tsh\t/bin/sh\nnot a line\n");
  assert.deepEqual([table.get(7), table.get(8), exes.get(7), exes.get(8)], [{ pid: 7, ppid: 1, start: 1.5e12, program: "python3 -c x" }, { pid: 8, ppid: 0, start: 0, program: "sh" }, undefined, "/bin/sh"]);
});

// This machine's own processes, read as XenoNet reads them on Linux and
// Windows: this process, its executable, its parent and its command line.
test("on Linux and Windows this process is read from the system itself", { skip: !["linux", "win32"].includes(process.platform) && "read natively on Linux and Windows" }, async () => {
  const procs = createProcs();
  assert.equal(procs.chains, true);
  const chain = await procs.chain(process.pid, { exe: true });
  assert.equal(chain[0].pid, process.pid);
  assert.ok(chain.length > 1, "and its parent");
  assert.ok(typeof chain[0].exe === "string" && chain[0].exe.length > 0, JSON.stringify(chain[0]));
});

// The live lookups work on macOS, Linux and Windows. One check end to end: a child process
// opens a connection to a local server, and procs resolves the server's
// socket back to that child's pid, with this test process among its
// ancestors.
test("peer() and chain() resolve a real connection to the process that made it", { skip: !["darwin", "linux", "win32"].includes(process.platform) }, async () => {
  const server = net.createServer();
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  const port = server.address().port;
  const conn = new Promise((r) => server.once("connection", r));
  // A child of this test process opens the connection and holds it open.
  const child = execFile(process.execPath, ["-e", `const s=require("net").connect(${port},"127.0.0.1",()=>{});setTimeout(()=>{},5000);`]);
  const socket = await conn;
  try {
    const procs = createProcs();
    const pid = await procs.peer(socket);
    assert.equal(pid, child.pid, "the socket resolves to the child that opened it");
    const chain = await procs.chain(pid);
    assert.ok(chain.some((p) => p.pid === process.pid), "this test process is one of its ancestors");
    assert.equal(await procs.alive({ pid, start: chain[0].start }), true);
    assert.equal(await procs.alive({ pid, start: chain[0].start + 1 }), false, "a different start time is a different process");
  } finally {
    child.kill();
    socket.destroy();
    server.close();
  }
});

// A stand-in for ps, netstat and codesign, which records what it was asked.
function fakeExec({ codesignOk = () => true } = {}) {
  const asked = [];
  let clock = 0;
  const ps = [
    "    1     0 Thu Oct  2 09:00:00 2026 /sbin/launchd",
    "  100     1 Thu Oct  2 09:00:01 2026 /Users/me/.local/bin/claude",
    "  200   100 Thu Oct  2 09:00:02 2026 /usr/local/bin/node",
    "  300   200 Thu Oct  2 09:00:03 2026 /bin/zsh",
  ].join("\n");
  const exec = async (file, args) => {
    asked.push([file, ...args]);
    if (file === "ps" && args[0] === "-axo") return ps;
    if (file === "ps") return "node /x/node_modules/@openai/codex/bin/codex.js";
    if (file === "codesign") {
      if (!codesignOk(args)) throw new Error("code object is not signed");
      return "";
    }
    return "";
  };
  return { asked, exec, now: () => clock, tick: (ms) => (clock += ms) };
}

test("chain(): a process and the ones above it, with Node told apart by what it runs", async () => {
  const fake = fakeExec();
  const procs = createProcs({ platform: "darwin", exec: fake.exec, now: fake.now });
  const chain = await procs.chain(300);
  assert.deepEqual(chain.map((p) => [p.pid, p.agent]), [[300, null], [200, "codex"], [100, "claude"]]);
  assert.deepEqual(await procs.chain(999), [], "a process that's gone has no chain");
  // The process table is shared for a moment, then read again.
  assert.equal(fake.asked.filter((a) => a[1] === "-axo").length, 1);
  await Promise.all([procs.snapshot(), procs.snapshot()]);
  assert.equal(fake.asked.filter((a) => a[1] === "-axo").length, 1);
  fake.tick(2000);
  await Promise.all([procs.snapshot(), procs.snapshot()]);
  assert.equal(fake.asked.filter((a) => a[1] === "-axo").length, 2, "one ps for both");
});

test("signed(): codesign asks the running process for its maker's team, once per process", async () => {
  const fake = fakeExec({ codesignOk: (args) => args[2] !== "666" });
  const procs = createProcs({ platform: "darwin", exec: fake.exec, now: fake.now });
  const claude = { pid: 100, start: 1, program: "/Users/me/.local/bin/claude", agent: "claude" };
  assert.equal(await procs.signed(claude), true);
  assert.equal(await procs.signed(claude), true);
  const asked = fake.asked.filter((a) => a[0] === "codesign");
  assert.equal(asked.length, 1, "asked once");
  assert.deepEqual(asked[0], ["codesign", "--verify", '-R=anchor apple generic and certificate leaf[subject.OU] = "Q6L2SF6YDW"', "100"]);
  assert.equal(await procs.signed({ ...claude, pid: 101, agent: "codex" }), true);
  assert.match(fake.asked.filter((a) => a[0] === "codesign")[1][2], /"2DC432GLL2"/, "Codex has OpenAI's team");
  assert.equal(await procs.signed({ ...claude, pid: 666 }), false, "not signed by its maker");
  assert.equal(await procs.signed({ pid: 5, start: 1, program: "/bin/zsh", agent: null }), false, "not an agent at all");
  // The cache is kept small.
  for (let pid = 1000; pid < 1502; pid++) await procs.signed({ ...claude, pid });
  const before = fake.asked.filter((a) => a[0] === "codesign").length;
  await procs.signed(claude);
  assert.equal(fake.asked.filter((a) => a[0] === "codesign").length, before + 1, "asked again after the cache was emptied");
});

test("selfTest(): a connection this process makes to itself comes back as this process", async () => {
  assert.deepEqual(await selfTest({ supported: false }), { supported: false });
  const ok = { supported: true, peer: async () => process.pid, chain: async (pid, { exe }) => [{ pid, ...(exe ? { exe: process.execPath } : {}) }] };
  assert.deepEqual(await selfTest(ok), { supported: true, peer: true, chain: true, exe: true });
  const noLsof = { supported: true, peer: async () => process.pid, chain: async (pid) => [{ pid, exe: null }] };
  assert.deepEqual(await selfTest(noLsof), { supported: true, peer: true, chain: true, exe: false }, "lsof didn't say what this process runs");
  const blind = { supported: true, peer: async () => null, chain: async () => assert.fail("no pid, no chain") };
  assert.deepEqual(await selfTest(blind), { supported: true, peer: false, chain: false, exe: false });
  const broken = { supported: true, peer: async () => { throw new Error("netstat failed"); }, chain: async () => [] };
  assert.deepEqual(await selfTest(broken), { supported: true, peer: false, chain: false, exe: false });
});
