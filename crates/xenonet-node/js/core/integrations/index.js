// core/integrations/index.js in Rust (crates/xenonet-integrations/src/
// index.rs): the integrations together. Their settings, the history and
// Pi's records, importing an export and the demo's made-up decider,
// OpenShell and records are the Rust's.
import path from "node:path";
import { native } from "../../index.js";
import { settle, createRecords } from "./common.js";
import { createRouting, routingInventory } from "./routing.js";
import { createOpenShell, executionProfiles } from "./openshell.js";
import { scanPi } from "./pi.js";
import { createReviews } from "./reviews.js";

export function createIntegrations({ dir = null, home, decider, nodes = () => [], demo = false, run, platform = process.platform, now = Date.now }) {
  const records = createRecords(dir ? path.join(dir, "records") : null, { now });
  const settingsFile = dir ? path.join(dir, "settings.json") : null;
  let piOn = native.intgPiOn(settingsFile);
  const demoDecider = { model: "demo/jev", status: async () => ({ key: "demo", model: "demo/jev" }), decide: async () => native.intgDemoDecision() };
  const routing = createRouting({ decider: demo ? demoDecider : decider, inventory: () => routingInventory(nodes()), records, now });
  const reviews = createReviews({ dir: dir ? path.join(dir, "reviews") : null, now, demo });
  const demoRun = async (_file, args) => {
    const r = native.intgDemoRun(args);
    if (r.error) throw new Error(r.error);
    return { stdout: r.ok };
  };
  const shell = createOpenShell({ records, now, platform, reviews, run: demo ? demoRun : run });
  if (demo) {
    const made = native.intgDemoRecords();
    records.add(made.decision);
    records.add(made.export);
    for (const [name, options] of made.sandboxes) records.add(native.intgDemoSandbox(name, options, reviews.seed(options)));
  }
  return {
    routing,
    shell,
    async status() {
      return { decision: await (demo ? demoDecider : decider).status(), piOn, piRoot: native.intgPiRootShown(String(home)), profiles: demo ? [{ backend: platform === "win32" ? "wsl" : "native", distro: "Demo-Linux" }] : await executionProfiles({ platform, run }), platform, demo };
    },
    history: (params) => native.intgHistory(records.list(), params),
    async pi(params) {
      const source = piOn && !demo ? await scanPi(native.intgPiRoot(String(home))) : { records: [], warnings: [] };
      return native.intgPi(records.list(), source, piOn, params);
    },
    enablePi(on) {
      const r = settle(native.intgSetPi(settingsFile, on));
      piOn = r.piOn;
      return r;
    },
    importPi(value) {
      const plan = settle(native.intgImportPlan(records.list(), value));
      return plan.add ? records.add(plan.add) : records.update(plan.update, plan.fields);
    },
  };
}
