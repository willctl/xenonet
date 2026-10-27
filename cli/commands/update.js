import { c, print } from "../lib/out.js";
import { confirm } from "./connect.js";
import { componentUpdates, updateComponent } from "../../core/components/updates.js";

export const help = "Check with xenonet update --check. xenonet update shows what would change and asks first (--yes to go ahead without asking); xenonet update tools, service or all installs those at once. The Tools update first, then the Service. If terminals are running, close them and run the service update again. Desktop updates are in the app's Updates panel.";

// Where a component goes in the order updates install: the Tools, then the
// Service, as the release notes ask, then anything else.
const RANK = { tools: 0, service: 1 };
const rank = (item) => RANK[item.component] ?? 2;

// The updates for `component` ("all" for every one), Tools first.
export const inOrder = (updates, component) => updates.filter((i) => component === "all" || i.component === component).sort((a, b) => rank(a) - rank(b));

// What an update would change, a line each: "Tools 0.5.0 → 0.5.3", or
// "Service (not installed) → 0.5.3". Those that are current say nothing.
const NAMES = { tools: "Tools", service: "Service" };
export const changes = (items) => items.filter((i) => i.available === true).map((i) => `${NAMES[i.component] ?? i.component} ${i.current ?? "(not installed)"} → ${i.version ?? "?"}`);

// A bare `xenonet update` shows what would change and asks first, unless
// --yes; naming what to update (tools, service or all) is the go-ahead.
export async function run(args, flags) {
  const named = args[0] || null;
  const component = named ?? "all";
  if (!["service", "tools", "all"].includes(component)) throw new Error(help);
  const items = inOrder(await componentUpdates(), component);
  if (!flags.check && !named) {
    const lines = changes(items);
    if (lines.length) {
      print("XenoNet would update:");
      for (const l of lines) print(`  ${l}`);
      print(c.dim("  Desktop updates itself, in its Updates panel."));
      if (!(await confirm("Update them?", flags))) return print(c.dim("Nothing changed."));
    }
  }
  for (const item of items) print(JSON.stringify(flags.check ? item : await updateComponent(item)));
}
