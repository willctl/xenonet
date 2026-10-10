// The doctor, wherever it's asked for (the xenonet command, the MCP tools): a
// running XenoNet runs it, since it knows its own sources and connections;
// with none running, it runs here, and the checks that need XenoNet are
// skipped. `only` keeps the checks whose id starts with one of its entries.
import { find, post, nameOf } from "../client.js";
import { runDoctor, summarize } from "./index.js";

export async function checkup({ only = null, ...opts } = {}) {
  const s = await find(opts);
  if (s) {
    const result = await post(s, "/api/doctor");
    const results = only ? result.results.filter((r) => only.some((o) => r.id === o || r.id.startsWith(`${o}.`))) : result.results;
    return { ...result, results, summary: summarize(results), by: nameOf(s) };
  }
  const [{ doctorContext }, { config }, { root, instanceDir, standsFor }] = await Promise.all([import("./context.js"), import("../config.js"), import("../workspace.js")]);
  // The folders of the XenoNet asked about: XenoNet Dev keeps its state.json
  // and drift.json apart from the installed XenoNet's. In them, the projects
  // folder it would read, the one you chose included, as state.json has
  // it: a running XenoNet's store isn't there to ask, and the doctor only
  // looks.
  const name = config.demo ? null : standsFor(opts);
  const paths = name ? { ...config, stateDir: instanceDir("state", name), cacheDir: instanceDir("cache", name) } : { ...config };
  return { ...(await runDoctor(doctorContext({ paths: { ...paths, projectsDir: root({ peek: true, stateDir: paths.stateDir }).path } }), { only })), by: null };
}
