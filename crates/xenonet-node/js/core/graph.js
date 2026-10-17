// core/graph.js in Rust (crates/xenonet-state/src/graph.rs): the hierarchy
// the page renders, from the projects, the sessions, Codex's own projects,
// the shared context's wiki, the shared systems and your layout. Which
// project orbits which is projects.rs's, worked out in Rust too.
import { native } from "../index.js";

export const activityOf = (updatedAt, now, windows) => native.activityOf(updatedAt, Number(now), windows);

export function buildGraph({ projectsDir, homeDir = null, projects, sessions, nativeProjects = [], resources = [], wiki = null, repos = new Map(), layout = {}, now = Date.now(), windows }) {
  return native.buildGraph({ projectsDir, homeDir, projects, sessions, nativeProjects, resources, wiki, repos: [...repos.entries()], layout, now, windows });
}
