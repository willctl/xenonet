#!/usr/bin/env node
// A stand-in for ssh, for tests: it takes the arguments a host link gives
// ssh, checks them the way a careful reader would, and runs the remote
// command on this machine, with XenoNet's own command line in place of the
// xenonet command, so a link can talk to a real relay with no SSH server.
//
//   fake-ssh.mjs -F <config> <options…> -- <alias> <command> <words…>
//
// It refuses (exit 255, as ssh does) anything a link mustn't send: no -F,
// options other than the link's, no -- before the alias, an alias the
// config doesn't define, or a remote word a shell would read as more than
// a word. FAKE_SSH_LOG, when set, gets a line of JSON per run.
import { spawn } from "node:child_process";
import fs from "node:fs";
import { otherReaders } from "../../core/private-file.js";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const args = process.argv.slice(2);
const refuse = (why) => {
  process.stderr.write(`fake-ssh: ${why}\n`);
  process.exit(255);
};

if (args[0] !== "-F" || !args[1]) refuse("no -F <config>");
const config = args[1];
const dashes = args.indexOf("--");
if (dashes < 0) refuse("no -- before the destination");
const options = args.slice(2, dashes);
const expected = ["-T", "-a", "-x", "-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=yes", "-o", "ClearAllForwardings=yes", "-o", "ConnectTimeout=10", "-o", "ServerAliveInterval=15", "-o", "ServerAliveCountMax=3", "-C"];
if (JSON.stringify(options) !== JSON.stringify(expected)) refuse(`unexpected options: ${options.join(" ")}`);
const [alias, command, ...words] = args.slice(dashes + 1);
let text = "";
try {
  text = fs.readFileSync(config, "utf8");
} catch {
  refuse(`can't read ${config}`);
}
if (!new RegExp(`^Host ${alias}$`, "m").test(text)) refuse(`ssh: Could not resolve hostname ${alias}: nodename nor servname provided, or not known`);
if (otherReaders(config).length) refuse(`Bad owner or permissions on ${config}`);
for (const w of [command, ...words]) if (!/^[A-Za-z0-9_./:=+-]+$/.test(w)) refuse(`a remote word a shell would read as more than a word: ${w}`);
if (command !== "xenonet" && !command.startsWith("/")) refuse(`bash: ${command}: command not found`);
if (process.env.FAKE_SSH_LOG) fs.appendFileSync(process.env.FAKE_SSH_LOG, `${JSON.stringify({ alias, command, words })}\n`);
if (process.env.FAKE_SSH_FAIL) {
  process.stderr.write(`${process.env.FAKE_SSH_FAIL}\n`);
  process.exit(Number(process.env.FAKE_SSH_CODE ?? 255));
}

const child = spawn(process.execPath, [path.join(root, "cli", "xenonet.js"), ...words], { stdio: "inherit" });
child.on("exit", (code, signal) => process.exit(signal ? 255 : (code ?? 255)));
for (const sig of ["SIGTERM", "SIGINT", "SIGHUP"]) process.on(sig, () => child.kill(sig));
