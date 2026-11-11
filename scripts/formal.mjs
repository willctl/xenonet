// The formal models (formal/README.md), on the Quint version pinned here,
// fetched by npx: no package dependency, since only this script and the
// formal CI job use it.
//
//   npm run formal                    for each model: typecheck, scenarios,
//                                     random runs of the design, and every
//                                     seeded bug found
//   npm run formal -- --model space   one model
//   npm run formal -- --samples 50000 --steps 60
//   npm run formal -- --model delivery --seed 0x1234
//                                     one run of a design, reproduced
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
export const QUINT = "@informalsystems/quint@0.33.1";

// Each model: its file, the module that is the design, its safety
// invariants (one by one, so a violation names the one it broke), the
// witnesses some run must reach, each seeded bug with the invariants that
// catch it, and its runs' defaults.
export const MODELS = {
  delivery: {
    file: "formal/delivery.qnt",
    invariants: ["atMostOnce", "perAuthorOrder", "gapDetected", "forkDetected", "compactEnds", "revokedRefused", "storedAfterReconnect"],
    witnesses: ["someFrozen", "someHeld", "someJump", "someRestoreLinked", "someRefused", "someServed"],
    // Taking an entry whose named predecessor isn't the one applied both
    // mixes two sides of a fork and breaks the chain of applied entries,
    // so either may fire first.
    bugs: {
      bugSkipGap: ["gapDetected"],
      bugNoPrevCheck: ["forkDetected", "gapDetected"],
      bugNoCompactEnd: ["compactEnds"],
      bugTimeRevocation: ["revokedRefused"],
      bugPruneNoRefuse: ["atMostOnce"],
    },
    samples: "20000",
    steps: "50",
  },
  space: {
    file: "formal/space.qnt",
    invariants: ["noWideningWithoutOwner", "curatorFromOwner", "agreement", "unshareStopsReads", "forkFreezes"],
    witnesses: ["someTakeWon", "someFrozen", "someCompeting", "someHealed", "someReshared", "someUnsharedAfterServing"],
    // A share or a take with no owner's signature widens the space or
    // names a curator the owner didn't, whichever a run reaches first.
    bugs: {
      bugNoOwnerCheck: ["noWideningWithoutOwner", "curatorFromOwner"],
      bugFirstTakeWins: ["agreement"],
      bugServeAfterUnshare: ["unshareStopsReads"],
      bugNoForkFreeze: ["forkFreezes"],
    },
    samples: "10000",
    steps: "40",
  },
};

function arg(name, fallback) {
  const i = process.argv.indexOf(name);
  return i === -1 ? fallback : process.argv[i + 1];
}

function quint(args) {
  const started = Date.now();
  const r = spawnSync("npx", ["--yes", QUINT, ...args], { cwd: root, encoding: "utf8", shell: process.platform === "win32", windowsHide: true });
  return { status: r.status, out: `${r.stdout ?? ""}${r.stderr ?? ""}`, secs: ((Date.now() - started) / 1000).toFixed(1) };
}

function step(label, args, check) {
  const r = quint(args);
  const failure = check(r);
  console.log(`${failure ? "FAIL" : "ok  "} ${label} (${r.secs} s)`);
  if (failure) {
    console.log(r.out.trimEnd());
    console.log(failure);
    process.exitCode = 1;
  }
  return r;
}

const passed = (r) => (r.status === 0 ? null : "quint failed");
const lines = (out, re) => out.split("\n").filter((l) => re.test(l)).map((l) => `     ${l.trim()}`).join("\n");

const only = arg("--model", null);
const seed = arg("--seed", null);
if (only && !MODELS[only]) {
  console.log(`no model ${only}: ${Object.keys(MODELS).join(", ")}`);
  process.exit(2);
}

for (const [name, m] of Object.entries(MODELS)) {
  if (only && name !== only) continue;
  const samples = arg("--samples", m.samples);
  const steps = arg("--steps", m.steps);
  const runArgs = (main) => ["run", m.file, `--main=${main}`, "--invariants", ...m.invariants, `--max-steps=${steps}`];
  console.log(`\n${m.file}`);
  step("typecheck", ["typecheck", m.file], passed);
  const t = step("scenarios", ["test", m.file, `--main=${name}`], passed);
  if (!process.exitCode) console.log(lines(t.out, /\bok\b/));
  const design = runArgs(name).concat("--verbosity=1", seed ? [`--seed=${seed}`] : [`--max-samples=${samples}`], "--witnesses", ...m.witnesses);
  // A witness no run reached means the runs never got to the states the
  // invariants are about, so they passed for nothing.
  const reached = (r) => passed(r) ?? (seed ? null : m.witnesses.filter((w) => !new RegExp(`${w} was witnessed in [1-9]`).test(r.out)).map((w) => `no run reached ${w}`).join("\n") || null);
  const d = step(`the design, ${seed ? `seed ${seed}` : `${samples} runs`} of ${steps} steps`, design, reached);
  if (!process.exitCode) console.log(lines(d.out, /witnessed|--seed/));
  if (seed) continue;
  for (const [bug, caught] of Object.entries(m.bugs)) {
    const r = step(`seeded bug ${bug} is found`, runArgs(bug).concat(`--max-samples=${samples}`, "--verbosity=2", "--hide", "s"), (r) => {
      if (r.status === 0) return `no run found ${bug}`;
      if (!caught.some((i) => r.out.includes(`❌ ${i}`))) return `${bug} broke an invariant other than ${caught.join(" or ")}`;
      return null;
    });
    console.log(lines(r.out, /❌|--seed/));
  }
}
