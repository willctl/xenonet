// The xenonet binary for a release (docs/design/rust-core.md, "Building for
// each platform"): built for one platform and CPU with Cargo's dist profile
// (whole-program optimized, no symbols), with no source path from
// this machine in it (the home folder, cargo's registry and the checkout
// remapped, and no debug info), ad hoc signed on macOS as the app is, and
// checked for the home folder's path as the packaged files are.
// XENONET_RUST_BIN names a binary built elsewhere (another runner, say)
// to use instead.
//
//   node scripts/rust-release.mjs [platform] [arch]     builds it, says where
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { cargo } from "./cargo.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

export const TARGETS = {
  "darwin-arm64": "aarch64-apple-darwin",
  "darwin-x64": "x86_64-apple-darwin",
  "win32-x64": "x86_64-pc-windows-msvc",
  "linux-x64": "x86_64-unknown-linux-gnu",
  "linux-arm64": "aarch64-unknown-linux-gnu",
};

export function rustTarget(platform, arch) {
  const target = TARGETS[`${platform}-${arch}`];
  if (!target) throw new Error(`XenoNet has no Rust build for ${platform}-${arch}`);
  return target;
}

export const binaryName = (platform) => (platform === "win32" ? "xenonet.exe" : "xenonet");

// The paths a binary mustn't carry, in each form they're written.
function personal(home = os.homedir()) {
  return [...new Set([home, home.replace(/\\/g, "/"), home.replace(/\\/g, "\\\\")])].filter((h) => h.length > 3);
}

export function checkBinary(file, { home = os.homedir() } = {}) {
  const bytes = fs.readFileSync(file);
  const hit = personal(home).find((h) => bytes.includes(Buffer.from(h)));
  if (hit) throw new Error(`${path.basename(file)} contains your home folder's path; build it with scripts/rust-release.mjs, which remaps it`);
}

// RUSTFLAGS that take this machine's paths out of what rustc writes.
export function remapFlags({ home = os.homedir(), cargoHome = process.env.CARGO_HOME || path.join(home, ".cargo"), checkout = root } = {}) {
  const pairs = [
    [path.join(cargoHome, "registry"), "/cargo/registry"],
    [path.join(cargoHome, "git"), "/cargo/git"],
    [checkout, "/xenonet"],
    [path.join(home, ".rustup"), "/rustup"],
    [home, "~"],
  ];
  return pairs.flatMap(([from, to]) => [...new Set([from, from.replace(/\\/g, "/")])].map((f) => `--remap-path-prefix=${f}=${to}`));
}

export function buildBinary({ platform = process.platform, arch = process.arch, env = process.env } = {}) {
  if (env.XENONET_RUST_BIN) {
    checkBinary(env.XENONET_RUST_BIN);
    return env.XENONET_RUST_BIN;
  }
  const target = rustTarget(platform, arch);
  const rustflags = [env.RUSTFLAGS, ...remapFlags()].filter(Boolean).join(" ");
  // The dist profile (Cargo.toml) has no debug info and no symbol table.
  cargo(["build", "--profile", "dist", "-p", "xenonet-cli", "--target", target], {
    stdio: "inherit",
    env: { ...env, RUSTFLAGS: rustflags },
  });
  const file = path.join(root, "target", target, "dist", binaryName(platform));
  // The dist profile asks rustc to strip, which on macOS runs the
  // toolchain's rust-objcopy; where that can't run (a toolchain without
  // its LLVM library, as on GitHub's Mac runners) rustc only warns and
  // leaves the symbol table, which names the build machine's folders.
  // Xcode's strip takes it out whatever happened, before the signature.
  if (platform === "darwin" && process.platform === "darwin") execFileSync("strip", [file], { stdio: "inherit" });
  // Ad hoc, as the app is: a certificate carries its owner's name.
  if (platform === "darwin" && process.platform === "darwin" && env.XENONET_SIGN !== "1") execFileSync("codesign", ["--sign", "-", "--force", file], { stdio: "inherit" });
  checkBinary(file);
  return file;
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const [platform = process.platform, arch = process.arch] = process.argv.slice(2);
  console.log(buildBinary({ platform, arch }));
}
