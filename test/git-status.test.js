// Repo status (core/git.js): each git project's uncommitted work, a few at
// a time, from git on a made-up machine (test/harness/procs.js), and what
// the watch's health says when git can't answer for one.
import { test } from "node:test";
import assert from "node:assert/strict";
import { RepoWatch, gitStatus, parseStatus, filterOverrides } from "../core/git.js";
import { fakeProcesses } from "./harness/procs.js";

// git status answers for the repos in `answers`, by folder; any other is
// "not a git repository". git config lists no filter drivers.
function machineWith(answers) {
  const machine = fakeProcesses();
  const asked = [];
  machine.program("git", ({ args }) => {
    const dir = args[args.indexOf("-C") + 1];
    if (!(dir in answers)) return { stderr: "fatal: not a git repository\n", code: 128 };
    if (args.includes("config")) return { code: 1 };
    asked.push(dir);
    return { stdout: answers[dir] };
  });
  return { machine, asked, watch: new RepoWatch({ status: (dir) => gitStatus(dir, { exec: machine.execFile }), available: () => true }) };
}

const project = (name, git = true) => ({ key: `/p/${name}`, path: `/p/${name}`, git: git ? { branch: "main" } : null });

test("each git project's status, read a few at a time; one git can't read keeps what it had, and the watch says so", async () => {
  const answers = { "/p/atlas": "## main...origin/main [ahead 2]\n M src/a.ts\n?? notes.md\n", "/p/ledger": "## fix/rounding\nM  ledger/reconcile.py\n" };
  const { watch, asked } = machineWith(answers);
  const projects = [project("atlas"), project("ledger"), project("broken"), project("plain", false)];
  const status = await watch.refresh(projects);
  assert.deepEqual(asked.sort(), ["/p/atlas", "/p/ledger"], "folders without git are left alone, and so is one git says isn't a repo");
  const { checkedAt, ...atlas } = status.get("/p/atlas");
  assert.deepEqual(atlas, { ...parseStatus(answers["/p/atlas"]) });
  assert.equal(typeof checkedAt, "number");
  assert.deepEqual([atlas.branch, atlas.ahead, atlas.modified, atlas.untracked, atlas.dirty], ["main", 2, 1, 1, 2]);
  assert.equal(status.has("/p/broken"), false);
  let h = watch.health();
  assert.deepEqual([watch.name, h.state, h.lastError.message, h.counters.repos, h.counters.reads, h.counters.passes], ["git-status", "degraded", "git couldn't read the status of /p/broken", 2, 3, 1]);

  // ledger stops answering: its last status stays. broken is fixed.
  delete answers["/p/ledger"];
  answers["/p/broken"] = "## main\n";
  await watch.refresh(projects);
  assert.equal(status.get("/p/ledger").branch, "fix/rounding");
  assert.equal(status.get("/p/broken").dirty, 0);
  assert.equal(watch.lastError.message, "git couldn't read the status of /p/ledger");
  answers["/p/ledger"] = "## fix/rounding\n";
  await watch.refresh(projects);
  h = watch.health();
  assert.deepEqual([h.state, h.counters.passes], ["running", 3]);
  assert.equal(status.get("/p/ledger").dirty, 0);
});

// What git config prints for --show-scope --get-regexp -z: scope, NUL,
// key, newline, value, NUL; git before 2.26 prints no scope.
const entry = (key, value) => (value == null ? key : `${key}\n${value}`) + "\0";
const listing = (entries) => entries.map(([scope, key, value]) => `${scope}\0${entry(key, value)}`).join("");
const unscoped = (entries) => entries.map(([, key, value]) => entry(key, value)).join("");
// git lfs install's driver, in your global config or (with --local) in the
// repo's own.
const lfs = (scope) => [
  [scope, "filter.lfs.clean", "git-lfs clean -- %f"],
  [scope, "filter.lfs.smudge", "git-lfs smudge -- %f"],
  [scope, "filter.lfs.process", "git-lfs filter-process"],
  [scope, "filter.lfs.required", "true"],
];

// git on a made-up machine whose config lists `drivers`: every run, with its
// arguments and environment. `old`: a git before 2.26, which doesn't know
// --show-scope.
function gitWith(drivers, { configCode = 0, old = false } = {}) {
  const machine = fakeProcesses();
  const runs = [];
  machine.program("git", ({ args, env }) => {
    runs.push({ args, env });
    if (args.includes("config")) {
      if (old && args.includes("--show-scope")) return { code: 129, stderr: "error: unknown option `show-scope'\n" };
      if (configCode) return { code: configCode, stderr: "fatal: bad config line 3 in file .git/config\n" };
      return drivers.length ? { stdout: old ? unscoped(drivers) : listing(drivers) } : { code: 1 };
    }
    return { stdout: "## main\n M notes.txt\n" };
  });
  return { runs, status: () => gitStatus("/p/unpacked", { exec: machine.execFile }) };
}

// The value each -c before the command sets, by key.
const settings = (args) => {
  const out = {};
  for (let i = 0; i < args.indexOf("status"); i++) if (args[i] === "-c") out[args[i + 1].slice(0, args[i + 1].indexOf("="))] = args[i + 1].slice(args[i + 1].indexOf("=") + 1);
  return out;
};
// The settings that empty the drivers `names`, each in turn, as
// [key, value].
const emptying = (names) => names.flatMap((name) => [["clean", ""], ["smudge", ""], ["process", ""], ["required", "false"]].map(([v, value]) => [`filter.${name}.${v}`, value]));

test("git status runs no program a repo's own config names: no fsmonitor, no hooks, no optional locks, no filters of its own but git-lfs's, nothing in submodules, no fetch", async () => {
  // An unpacked repo whose .git/config has filters of its own (one in a
  // file it includes, with a name git allows) and its config.worktree
  // another, beside your own drivers: git-lfs's and another, in your
  // global and system config.
  const git = gitWith([
    ...lfs("global"),
    ["global", "filter.crypt.clean", "git-crypt clean"],
    ["global", "filter.crypt.smudge", "git-crypt smudge"],
    ["system", "filter.media.process", "media-filter --process"],
    ["local", "filter.marker.clean", "sh -c 'touch ran; cat'"],
    ["local", "filter.marker.process", "sh -c 'touch ran'"],
    ["local", "filter.my driver.v2.clean", "cat"],
    ["worktree", "filter.wt.smudge", "sh -c 'touch ran'"],
    ["local", "lfs.repositoryformatversion", "0"],
  ]);
  assert.deepEqual(await git.status().then((s) => [s.branch, s.modified]), ["main", 1]);
  const [config, status] = git.runs;
  // The drivers and where each comes from, from git config, which reads
  // files and runs nothing; an include's come in the scope of the file
  // that includes it (no --local, which leaves includes out).
  assert.deepEqual(config.args, ["-C", "/p/unpacked", "config", "--show-scope", "--get-regexp", "-z", "^(filter|lfs)\\."]);
  const args = status.args;
  assert.deepEqual(args.slice(0, 2), ["-C", "/p/unpacked"]);
  const at = args.indexOf("status");
  assert.ok(args.indexOf("--no-optional-locks") > 0 && args.indexOf("--no-optional-locks") < at, "no index rewritten, so no post-index-change hook");
  // A setting on the command line wins over the repo's, and comes before
  // the command it's for.
  assert.deepEqual(settings(args), { "core.fsmonitor": "false", "core.hooksPath": "/dev/null", ...Object.fromEntries(emptying(["marker", "my driver.v2", "wt"])) });
  assert.ok(!args.some((a) => /^filter\.(lfs|crypt|media)\./.test(a)), "your own drivers run as they do anywhere");
  assert.deepEqual(args.slice(at), ["status", "--porcelain=v1", "-b", "--untracked-files=normal", "--ignore-submodules=dirty"]);
  assert.ok(git.runs.every((r) => r.env.GIT_NO_LAZY_FETCH === "1"), "no objects fetched for a partial clone");

  // No drivers at all (git config exits 1): the same guards, no filters.
  const plain = gitWith([]);
  assert.equal((await plain.status()).branch, "main");
  assert.deepEqual(settings(plain.runs[1].args), { "core.fsmonitor": "false", "core.hooksPath": "/dev/null" });
});

test("a git before 2.26 lists no scopes: every driver counts as the repo's, but git-lfs's", async () => {
  const git = gitWith([...lfs("global"), ["global", "filter.crypt.clean", "git-crypt clean"], ["local", "filter.marker.clean", "sh -c 'touch ran'"]], { old: true });
  assert.equal((await git.status()).branch, "main");
  assert.deepEqual(git.runs.map((r) => r.args.slice(2, 4)), [["config", "--show-scope"], ["config", "--get-regexp"], ["--no-optional-locks", "-c"]], "asked again without --show-scope, then status");
  assert.deepEqual(settings(git.runs[2].args), { "core.fsmonitor": "false", "core.hooksPath": "/dev/null", ...Object.fromEntries(emptying(["crypt", "marker"])) });
});

test("the repo's own drivers are emptied, but one whose commands there are git-lfs's alone; yours stay, unless the repo gives git-lfs extensions", () => {
  const emptied = (entries) => [...new Set(filterOverrides(listing(entries)).filter((a) => a !== "-c").map((a) => a.slice(7, a.lastIndexOf("."))))];
  assert.deepEqual(emptied(lfs("local")), [], "git lfs install --local");
  assert.deepEqual(emptied([["local", "filter.lfs.smudge", "git-lfs smudge --skip -- %f"], ["local", "filter.lfs.process", "git-lfs filter-process --skip"]]), [], "installed with --skip-smudge");
  assert.deepEqual(emptied([...lfs("local").slice(0, 2), ["local", "filter.lfs.process", "git-lfs filter-process; sh -c 'touch ran'"]]), ["lfs"], "one command of its own");
  assert.deepEqual(emptied([...lfs("global"), ["local", "filter.lfs.clean", "sh -c 'touch ran'"]]), ["lfs"], "git-lfs's in your config, another in the repo's, which wins");
  assert.deepEqual(emptied([...lfs("global"), ["local", "filter.lfs.required", "false"]]), [], "a repo that only makes yours not required runs nothing");
  assert.deepEqual(emptied([["local", "filter.lfs.process", null]]), ["lfs"], "a key with no value");
  assert.deepEqual(emptied([["local", "filter.lfs-copy.clean", "git-lfs clean -- %f"], ["local", "filter..clean", "cat"], ["local", "filter.Mixed.Case.process", "cat"], ["local", "filter.clean", "cat"]]), ["", "Mixed.Case"], "by its commands, not its name; [filter] alone names no driver");

  // Yours, whatever they run: your system and global config, and a -c.
  assert.deepEqual(emptied([["global", "filter.crypt.clean", "git-crypt clean"], ["system", "filter.media.smudge", "media-filter"], ["command", "filter.x.clean", "cat"]]), []);
  // Any other scope counts as the repo's, to be safe.
  assert.deepEqual(emptied([["unknown", "filter.x.clean", "cat"]]), ["x"]);
  // Git-lfs runs an extension's commands through any driver that runs it.
  assert.deepEqual(emptied([...lfs("global"), ["global", "filter.crypt.clean", "git-crypt clean"], ["local", "lfs.Extension.marker.clean", "sh -c 'touch ran'"]]), ["lfs", "crypt"], "the repo's extensions empty every driver, yours too");
  assert.deepEqual(emptied([...lfs("local"), ["global", "lfs.extension.mine.clean", "my-lfs-extension %f"]]), [], "yours are yours");
  assert.deepEqual(filterOverrides(""), []);
  // A listing with no scopes (git before 2.26) is all the repo's.
  assert.deepEqual(filterOverrides(unscoped([...lfs("global"), ["global", "filter.x.clean", "cat"]]), { scoped: false }), emptying(["x"]).flatMap(([k, v]) => ["-c", `${k}=${v}`]));
});

test("a repo whose drivers git can't list, or whose driver -c can't name, gets no status", async () => {
  const broken = gitWith([], { configCode: 128 });
  assert.equal(await broken.status(), null);
  assert.equal(broken.runs.length, 1, "status never ran");
  // git -c splits at the first =, so it can't empty filter.x=y: git would
  // read the setting as filter.x's.
  const odd = gitWith([["local", "filter.x=y.clean", "sh -c 'touch ran'"]]);
  assert.equal(await odd.status(), null);
  assert.equal(odd.runs.length, 1);
  assert.equal(filterOverrides(listing([["local", "filter.x=y.clean", "cat"]])), null);
  // Yours can keep such a name: it isn't emptied, so it needn't be named.
  assert.deepEqual(filterOverrides(listing([["global", "filter.x=y.clean", "cat"]])), []);
});

test("without git, nothing is asked and nothing changes", async () => {
  const machine = fakeProcesses();
  const watch = new RepoWatch({ status: (dir) => gitStatus(dir, { exec: machine.execFile }), available: () => false });
  assert.equal((await watch.refresh([project("atlas")])).size, 0);
  assert.deepEqual([machine.spawned.length, watch.state], [0, "idle"]);
});
