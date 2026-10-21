// core/config.js in Rust (crates/xenonet-app/src/config.rs): the settings
// the service runs with, from its command line and environment. What only
// JS can find is asked for through the functions passed in: the demo's
// made-up world, Xenolith's (or Agent Wiki's) folder and server, and Claude
// desktop's folder.
import os from "node:os";
import { native } from "../index.js";
import { peer } from "../peer.js";
import { appDir, findClaudeDesktopDir } from "./dirs.js";

const { createWorld } = await import(peer("demo.js"));
const { contextServerOf, findContextDir } = await import(peer("xenolith.js"));

export { appDir };

const made = native.appConfig(
  process.argv,
  { ...process.env },
  os.homedir(),
  process.platform,
  () => createWorld(),
  () => findContextDir() ?? null,
  (dir) => contextServerOf(dir) ?? null,
  () => findClaudeDesktopDir() ?? null,
);

export const config = { ...made, readers: new Set(made.readers), hiddenResources: new Set(made.hiddenResources) };
