// core/doctor/checkup.js in Rust (crates/xenonet-doctor/): the doctor,
// wherever it's asked for. Which results `only` keeps and the summary are
// the Rust's; finding a running XenoNet and its folders are here.
import { native } from "../../index.js";
import { peer } from "../../peer.js";
import { runDoctor, summarize } from "./index.js";

const { find, post, nameOf } = await import(peer("client.js"));

export async function checkup({ only = null, ...opts } = {}) {
  const s = await find(opts);
  if (s) {
    const result = await post(s, "/api/doctor");
    const results = only ? result.results.filter((r) => native.doctorKept(String(r.id), only.map(String))) : result.results;
    return { ...result, results, summary: summarize(results), by: nameOf(s) };
  }
  const [{ doctorContext }, { config }, { root, instanceDir, standsFor }] = await Promise.all([import("./context.js"), import(peer("config.js")), import(peer("workspace.js"))]);
  const name = config.demo ? null : standsFor(opts);
  const paths = name ? { ...config, stateDir: instanceDir("state", name), cacheDir: instanceDir("cache", name) } : { ...config };
  return { ...(await runDoctor(doctorContext({ paths: { ...paths, projectsDir: root({ peek: true, stateDir: paths.stateDir }).path } }), { only })), by: null };
}
