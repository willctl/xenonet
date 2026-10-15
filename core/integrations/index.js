import fs from "node:fs";
import path from "node:path";
import { createRecords, clean, fail, queryRecords } from "./common.js";
import { createRouting, routingInventory } from "./routing.js";
import { createOpenShell, executionProfiles } from "./openshell.js";
import { scanPi, piExportMetadata } from "./pi.js";
import { createReviews } from "./reviews.js";

export function createIntegrations({ dir = null, home, decider, nodes = () => [], demo = false, run, platform = process.platform, now = Date.now }) {
  const records = createRecords(dir ? path.join(dir, "records") : null, { now });
  const settingsFile = dir ? path.join(dir, "settings.json") : null;
  let piOn = false;
  try { piOn = JSON.parse(fs.readFileSync(settingsFile, "utf8")).piOn === true; } catch {}
  const demoDecider = { model: "demo/jev", status: async () => ({ key: "demo", model: "demo/jev" }), decide: async () => ({ answers: { tool: { type: "choice", choice: "codex", confidence: 0.93, probabilities: { codex: 0.8, claude: 0.15, review: 0.05 } } }, model: "demo/jev", cost: 0, ms: 12 }) };
  const routing = createRouting({ decider: demo ? demoDecider : decider, inventory: () => routingInventory(nodes()), records, now });
  const reviews = createReviews({ dir: dir ? path.join(dir, "reviews") : null, now, demo });
  const shell = createOpenShell({ records, now, platform, reviews, run: demo ? async (_file, args) => {
    if (args.includes("-e")) {
      if (args.includes("demo-unavailable")) throw new Error("Demo gateway is offline.");
      return { stdout: JSON.stringify({ files: [{ path: "hello.txt", content: Buffer.from("Hello from the sandbox.\n").toString("base64") }], omissions: [], problems: args.includes("demo-partial") ? [{ path: "image.png", reason: "Binary file" }] : [] }) };
    }
    if (args.includes("get") && args.includes("json")) return { stdout: JSON.stringify({ phase: "Ready" }) };
    if (args.includes("policy")) return { stdout: "Demo policy: /workspace writable; network limited to the selected provider." };
    if (args.includes("logs")) return { stdout: "Demo execution-policy-denial: unlisted.example was blocked. This is a runtime policy event, not a model refusal." };
    return { stdout: args.includes("--version") ? "OpenShell demo" : '{"sandboxes":[]}' };
  } : run });
  if (demo) {
    records.add({ kind: "decision", status: "needs_confirmation", selected: "codex", model: "demo/jev", taskHash: "demo", cost: 0, ms: 12, reason: null, candidates: [{ id: "codex", name: "Codex", probability: 0.8, filtered: false, confirmation: true }], outcome: "useful" });
    records.add({ kind: "pi-export", sourceId: "pi-export:demo", externalId: "demo-pi-run", status: "completed", model: "demo-model", provider: "demo", tokens: 420, cost: 0, artifacts: 2, checkpoints: 1, diagnostics: 0 });
    for (const [name, options] of [
      ["changes", { before: { "hello.txt": "Hello.\n", "removed.txt": "Old example.\n" }, after: { "hello.txt": "Hello from the sandbox.\n", "new.txt": "New example.\n" } }],
      ["no-changes", { before: { "hello.txt": "Hello.\n" }, after: { "hello.txt": "Hello.\n" } }],
      ["source-drift", { before: { "hello.txt": "Hello.\n" }, after: { "hello.txt": "Hello from the sandbox.\n" }, sourceState: "changed" }],
      ["partial", { before: { "hello.txt": "Hello.\n" }, after: { "hello.txt": "Hello from the sandbox.\n" }, problems: [{ path: "image.png", reason: "Binary file" }] }],
      ["unavailable", { before: { "hello.txt": "Hello.\n" }, after: {}, unavailable: true }],
    ]) records.add({ kind: "sandbox", status: "created", name: `demo-${name}`, agent: "pi", image: "demo/pi", profile: { backend: "native", distro: "" }, reviewId: reviews.seed(options), execution: "unknown", completedSteps: 5, launchSteps: 5, retrieval: options.unavailable ? "unavailable" : options.problems ? "incomplete" : "retrieved" });
  }
  return {
    routing, shell,
    async status() { return { decision: await (demo ? demoDecider : decider).status(), piOn, piRoot: clean(path.join(home, ".pi", "agent", "sessions"), 1024), profiles: demo ? [{ backend: platform === "win32" ? "wsl" : "native", distro: "Demo-Linux" }] : await executionProfiles({ platform, run }), platform, demo }; },
    history(params) { return queryRecords(records.list().filter((r) => r.kind === "decision" || r.kind === "sandbox"), params); },
    async pi(params) {
      const source = piOn && !demo ? await scanPi(path.join(home, ".pi", "agent", "sessions")) : { records: [], warnings: [] };
      return { ...queryRecords([...records.list().filter((r) => r.kind === "pi-export"), ...source.records], params), enabled: piOn, warnings: source.warnings };
    },
    enablePi(on) {
      if (typeof on !== "boolean") fail("Choose whether to read Pi sessions.");
      if (settingsFile) {
        const tmp = `${settingsFile}.tmp`;
        fs.writeFileSync(tmp, JSON.stringify({ piOn: on }), { mode: 0o600 });
        fs.renameSync(tmp, settingsFile);
      }
      piOn = on;
      return { piOn };
    },
    importPi(value) {
      const metadata = piExportMetadata(value);
      const { id: sourceId, ...fields } = metadata;
      const existing = records.list().find((r) => r.sourceId === sourceId);
      return existing ? records.update(existing.id, fields) : records.add({ ...fields, sourceId });
    },
  };
}
