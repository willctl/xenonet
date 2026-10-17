// core/actors/demo.js in Rust (crates/xenonet-actors/src/demo.rs): the demo
// world's actors and their scripts. The world's own nodes (its projects,
// sessions and files, and Wren's Claude account) are core/demo.js's, read
// here.
import path from "node:path";
import { world, demoSessionList, demoClaudeAccount } from "../../../../../core/demo.js";
import { claudeId } from "../../../../../core/identities.js";
import { pathKey } from "../paths.js";
import { native } from "../../index.js";

export const STEP_MS = native.actorConstants().stepMs;

// A node of the demo world, by what it is.
export function resolveDemo(kind, what) {
  const parts = String(what).split("/");
  if (kind === "project") return `project:${pathKey(path.join(world.projectsDir, ...parts))}`;
  if (kind === "session") return demoSessionList().find((s) => s.title === what)?.id ?? null;
  if (kind === "file") return path.join(world.projectsDir, ...parts);
  if (kind === "identity") return claudeId(demoClaudeAccount(what).account, demoClaudeAccount(what).org);
  return null;
}

const resolver = (op, arg) => (op === "resolve" ? resolveDemo(arg[0], arg[1]) : undefined);

export const DEMO_ACTORS = () => native.demoActors(resolver);
export const DEMO_SCRIPTS = () => native.demoScripts(resolver);

// Adds the demo's actors to `actors`, with the last few hours of their
// scripts behind them, and drives the scripts on while it runs: step(),
// which plays the next turn.
const DEMO = Symbol.for("xenonet.actors.demo");
export const addDemoActors = (actors, { history = 3 * 3600e3 } = {}) => actors[DEMO](history);
