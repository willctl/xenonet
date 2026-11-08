// Runs this checkout's xenonet binary with the arguments given: the newer of
// target/release/xenonet and target/debug/xenonet, as XenoNet Dev does,
// building target/debug/xenonet first when there's neither. The
// xenonet-console MCP server in .mcp.json and .codex/config.toml starts
// this way (`node scripts/xenonet.mjs mcp console`), so it needs no path to
// a build.
//
//   node scripts/xenonet.mjs console launch shot:overview
//   node scripts/xenonet.mjs mcp console
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { buildBinary } from "./rust-bin.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

// The newer build, or null when there's none.
export function builtBinary() {
  const name = process.platform === "win32" ? "xenonet.exe" : "xenonet";
  const built = ["release", "debug"].map((profile) => path.join(root, "target", profile, name)).filter((f) => fs.existsSync(f));
  return built.sort((a, b) => fs.statSync(b).mtimeMs - fs.statSync(a).mtimeMs)[0] ?? null;
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  // An MCP server's stdout is its protocol, so cargo's output goes to stderr.
  const bin = builtBinary() ?? buildBinary({ stdio: ["ignore", 2, "inherit"] });
  const child = spawn(bin, process.argv.slice(2), { stdio: "inherit", windowsHide: true });
  for (const signal of ["SIGINT", "SIGTERM"]) process.on(signal, () => child.kill(signal));
  child.on("exit", (code, signal) => (signal ? process.kill(process.pid, signal) : process.exit(code ?? 1)));
}
