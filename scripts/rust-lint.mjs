// The Rust core's static analysis (docs/design/rust-core.md, "Static
// analysis"), its layering and Xenolith's lockfile beside it, each check in
// turn, as CI runs them. Stops
// at the end, not at the first failure, and exits 1 if any failed.
//
//   npm run lint:rust            every check
//   npm run lint:rust -- --fix   cargo fmt and cargo shear --fix first
import { spawnSync } from "node:child_process";
import { cargoCommand } from "./cargo.mjs";

const fix = process.argv.includes("--fix");
const checks = [
  ["format", ["fmt", "--all", fix ? "" : "--check"].filter(Boolean)],
  ["clippy", ["clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings"]],
  ["docs", ["doc", "--workspace", "--no-deps", "--locked"], { RUSTDOCFLAGS: "-D warnings" }],
  // Advisories, licenses, bans and sources, from deny.toml.
  ["dependencies", ["deny", "--locked", "check"], {}, "cargo install --locked cargo-deny"],
  // Xenolith's lock, under the same deny.toml.
  ["Xenolith's dependencies", ["deny", "--manifest-path", "xenolith/rust/Cargo.toml", "--config", "deny.toml", "--locked", "check"], {}, "cargo install --locked cargo-deny"],
  ["unused dependencies", ["shear", ...(fix ? ["--fix"] : [])], {}, "cargo install --locked cargo-shear"],
];

let failed = 0;
// The core's layering (scripts/rust-layers.mjs): no core crate depends on a
// surface or an adapter crate, or names a platform by its id, and each crate
// keeps to its class's limit on internal dependencies. Then the two
// lockfiles (scripts/xenolith-lock.mjs): Xenolith builds XenoNet's libraries
// on the packages the root's lock tests them on.
const scripts = [
  ["layering", "rust-layers.mjs"],
  ["Xenolith's lockfile", "xenolith-lock.mjs"],
];
for (const [name, file] of scripts) {
  process.stderr.write(`\n== ${name}: node scripts/${file}\n`);
  const run = spawnSync(process.execPath, [new URL(`./${file}`, import.meta.url).pathname], { stdio: "inherit", windowsHide: true });
  if (run.status !== 0) failed++;
}
for (const [name, args, env = {}, install] of checks) {
  const [file, argv, environment] = cargoCommand(args, { ...process.env, CARGO_PROFILE_DEV_DEBUG: "0", ...env });
  process.stderr.write(`\n== ${name}: cargo ${args.join(" ")}\n`);
  const run = spawnSync(file, argv, { stdio: "inherit", env: environment, windowsHide: true });
  if (run.status === 0) continue;
  failed++;
  if (install && run.status === 101) process.stderr.write(`(if cargo ${args[0]} isn't installed: ${install})\n`);
}
const total = checks.length + scripts.length;
process.stderr.write(failed ? `\n${failed} of ${total} checks failed.\n` : `\nAll ${total} checks pass.\n`);
process.exitCode = failed ? 1 : 0;
