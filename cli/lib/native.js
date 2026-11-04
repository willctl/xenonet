// Hands a command to the xenonet binary, for the commands that exist only
// there (COMMANDS' native ones). It runs the binary XENONET_CLI_BIN names,
// else the newer of target/release and target/debug in this checkout.
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { Fail } from "./out.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");

export function nativeBinary({ env = process.env, platform = process.platform } = {}) {
  if (env.XENONET_CLI_BIN) return env.XENONET_CLI_BIN;
  const name = platform === "win32" ? "xenonet.exe" : "xenonet";
  const found = ["release", "debug"]
    .map((dir) => path.join(root, "target", dir, name))
    .filter((file) => fs.existsSync(file))
    .sort((a, b) => fs.statSync(b).mtimeMs - fs.statSync(a).mtimeMs);
  return found[0] ?? null;
}

export function delegate(argv, { bin = nativeBinary() } = {}) {
  if (!bin) throw new Fail("this command is in the xenonet binary, which isn't built here: npm run rust:bin", 1);
  const run = spawnSync(bin, argv, { stdio: "inherit" });
  if (run.error) throw new Fail(`couldn't run ${bin}: ${run.error.message}`, 1);
  return run.status ?? 1;
}
