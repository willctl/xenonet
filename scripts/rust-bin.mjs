// Builds the xenonet binary, the service and the command, from this
// checkout: target/debug/xenonet, or target/release/xenonet with --release.
// XenoNet Dev and npm run dev run the newer of the two
// (electron/service-runtime.js), and XenoNet Dev restarts its service when
// the one it runs is rebuilt.
//
//   npm run rust:bin
//   npm run rust:bin -- --release
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { cargo } from "./cargo.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

export function buildBinary({ release = false, stdio = "inherit" } = {}) {
  cargo(["build", "-p", "xenonet-cli", ...(release ? ["--release"] : [])], { stdio, env: { ...process.env, CARGO_PROFILE_DEV_DEBUG: process.env.CARGO_PROFILE_DEV_DEBUG ?? "0" } });
  return path.join(root, "target", release ? "release" : "debug", process.platform === "win32" ? "xenonet.exe" : "xenonet");
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    buildBinary({ release: process.argv.includes("--release") });
  } catch (err) {
    // cargo has said why.
    process.exitCode = err.status ?? 1;
  }
}
