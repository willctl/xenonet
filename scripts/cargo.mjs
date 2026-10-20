// cargo on the toolchain rust-toolchain.toml pins. `npm run cargo -- <args>`
// runs it as a plain command would, on the pin (cargo test --workspace). rustup's own cargo
// reads that file, but a PATH can name a toolchain's cargo directly, which
// doesn't; `rustup run` then picks the pinned one. Without rustup, the
// cargo on the PATH it is.
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

export function pinned() {
  try {
    return /^\s*channel\s*=\s*"([^"]+)"/m.exec(fs.readFileSync(path.join(root, "rust-toolchain.toml"), "utf8"))?.[1] ?? null;
  } catch {
    return null;
  }
}

const hasRustup = () => {
  try {
    execFileSync("rustup", ["--version"], { stdio: "ignore", windowsHide: true });
    return true;
  } catch {
    return false;
  }
};

// The pinned toolchain's folder of programs, or null. rustup run doesn't
// always put it first on the PATH, and cargo then runs the rustc it finds.
function toolchainBin(channel) {
  try {
    return path.dirname(execFileSync("rustup", ["which", "cargo", "--toolchain", channel], { encoding: "utf8", windowsHide: true }).trim());
  } catch {
    return null;
  }
}

// The command that runs cargo with `args`, as [file, args, env].
export function cargoCommand(args, env = process.env) {
  const channel = pinned();
  const bin = channel && hasRustup() ? toolchainBin(channel) : null;
  if (!bin) return ["cargo", args, env];
  return [path.join(bin, process.platform === "win32" ? "cargo.exe" : "cargo"), args, { ...env, PATH: `${bin}${path.delimiter}${env.PATH ?? ""}`, RUSTC: path.join(bin, process.platform === "win32" ? "rustc.exe" : "rustc") }];
}

export function cargo(args, options = {}) {
  const [file, argv, env] = cargoCommand(args, options.env ?? process.env);
  return execFileSync(file, argv, { cwd: root, windowsHide: true, ...options, env });
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  try {
    cargo(process.argv.slice(2), { stdio: "inherit" });
  } catch (err) {
    process.exitCode = err.status ?? 1;
  }
}
