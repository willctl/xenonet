// core/delivery/setup.js in Rust (crates/xenonet-engine/src/delivery/setup.rs):
// what live delivery needs on this machine, for xenonet doctor --live. The
// commands, files and OpenCode servers are the twin's.
import { execFile } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import { native } from "../../index.js";
import { drive, calls } from "../../drive.js";
import { createOpenCode } from "./opencode.js";

const run = (cmd, args) => new Promise((resolve) => execFile(cmd, args, { timeout: 5000, windowsHide: true, shell: process.platform === "win32" }, (err, out) => resolve(err ? null : String(out).trim())));

export const CHANNELS_FROM = [2, 1, 80];
export const newer = (v, min) => native.setupNewer(v, [...min].map(Number));

export const serverName = (port) => native.setupServerName(port);

export const LISTEN_SECONDS = 300;
export const listenSettings = ({ port = 47840, seconds = LISTEN_SECONDS } = {}) => native.setupListenSettings(port, seconds);

export const writeListenSettings = ({ dir, port = 47840, seconds = LISTEN_SECONDS }) => native.setupWriteListen({ fs }, String(dir), port, seconds);

export async function liveSetup({ port = 47840, exec = run, opencode = createOpenCode(), listenFile = null, home = os.homedir() } = {}) {
  return drive(native.setupLive(port, listenFile, home), calls({ exec: (cmd, args) => exec(cmd, args), servers: () => opencode.servers() }));
}
