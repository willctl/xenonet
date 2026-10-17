// cargo on the toolchain rust-toolchain.toml pins. rustup's own cargo
// reads that file, but a PATH can name a toolchain's cargo directly, which
// doesn't; `rustup run` then picks the pinned one. Without rustup, the
// cargo on the PATH it is.
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

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

// The command that runs cargo with `args`, as [file, args].
export function cargoCommand(args) {
  const channel = pinned();
  return channel && hasRustup() ? ["rustup", ["run", channel, "cargo", ...args]] : ["cargo", args];
}

export function cargo(args, options = {}) {
  const [file, argv] = cargoCommand(args);
  return execFileSync(file, argv, { cwd: root, windowsHide: true, ...options });
}
