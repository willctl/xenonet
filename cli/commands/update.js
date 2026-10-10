import { print } from "../lib/out.js";
import { componentUpdates, updateComponent } from "../../core/components/updates.js";

export const help = "Check with xenonet update --check. Run xenonet update tools, service or all to install. If terminals are running, close them and run the service update again. Desktop updates are in the app's Updates panel.";
export async function run(args, flags) {
  const component = args[0] || "all";
  if (!["service", "tools", "all"].includes(component)) throw new Error(help);
  const updates = await componentUpdates();
  for (const item of updates.filter((i) => component === "all" || i.component === component)) print(JSON.stringify(flags.check ? item : await updateComponent(item)));
}
