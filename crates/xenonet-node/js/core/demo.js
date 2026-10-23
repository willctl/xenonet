// core/demo.js in Rust (crates/xenonet-app/src/demo.rs): demo mode, a
// made-up workspace that runs through XenoNet's real pipeline: projects
// (real git repos), Claude Code transcripts, Codex rollouts and a small
// shared context in a temp folder, and live sessions that keep moving. Your real
// projects and sessions are never read while it runs. Whether git runs and
// the ids of accounts come from the modules that own them.
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import { execFile } from "node:child_process";
import { native } from "../index.js";
import { peer } from "../peer.js";
import { settle } from "../settle.js";

const { gitAvailable } = await import(peer("git.js"));
const { claudeId, gitId, githubId } = await import(peer("identities.js"));

// One world per port, so demo servers running side by side don't wipe each
// other's.
const demo = new native.AppDemo(fs.realpathSync.native(os.tmpdir()), String(process.env.XENONET_PORT ?? 47841), new Date().toLocaleDateString("en-CA"));

export const world = demo.world();

const local = (ms) => {
  const d = new Date(ms);
  return [d.getFullYear(), d.getMonth() + 1, d.getDate(), d.getHours(), d.getMinutes()];
};

// Math.random, crypto.randomUUID and Date.now, looked up as they're used,
// so a test that seeds or fixes them reaches the Rust too.
const random = () => Math.random();
const uuid = () => crypto.randomUUID();
const clock = () => Date.now();

let clients = null;

// Builds the demo workspace from scratch. Synchronous, so config can point
// at it before anything else starts.
export function createWorld(now = Date.now()) {
  Object.assign(world, settle(demo.createWorld(Number(now), process.platform, () => !!gitAvailable(), local, claudeId, githubId, gitId, random, uuid, clock)));
  clients = null;
  return world;
}

// The second folder's repos are made the first time something asks for
// that folder, side by side, so the demo starts as fast as it did without.
export function clientsReady() {
  const run = (dir, args) => new Promise((resolve, reject) => execFile("git", args, { cwd: dir }, (err) => (err ? reject(err) : resolve())));
  clients ??= Promise.all(
    demo.clientRepos().map(async ({ dir, steps, dirty }) => {
      try {
        for (const args of steps) await run(dir, args);
        for (const [file, text] of dirty) fs.writeFileSync(file, text);
      } catch {} // No git: the projects still show, without repo status.
    }),
  );
  return clients;
}

export const demoClaudeAccount = (project) => native.appDemoClaudeAccount(project == null ? null : String(project));

export const demoSessions = (app) => settle(demo.sessions(String(app)));

export const demoSessionList = () => demo.sessionList();

export const demoNativeProjects = () => demo.nativeProjects();

export const demoTranscript = (rawId) => settle(demo.transcript(String(rawId)));

export const demoPicks = (outline) => native.appDemoPicks(outline);

export const demoNarration = () => native.appDemoNarration();

// Keeps the live sessions moving: one step every couple of seconds, round
// robin.
export function startDriver() {
  let k = 0;
  const timer = setInterval(() => demo.stepLive(k++, Date.now(), random, uuid, clock), 2200);
  return () => clearInterval(timer);
}
