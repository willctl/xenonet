// The Rust core's static analysis (docs/design/rust-core.md, "Static
// analysis"), each check in turn, as CI runs them. Stops at the end, not at
// the first failure, and exits 1 if any failed.
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
  ["unused dependencies", ["shear", ...(fix ? ["--fix"] : [])], {}, "cargo install --locked cargo-shear"],
];

let failed = 0;
for (const [name, args, env = {}, install] of checks) {
  const [file, argv, environment] = cargoCommand(args, { ...process.env, CARGO_PROFILE_DEV_DEBUG: "0", ...env });
  process.stderr.write(`\n== ${name}: cargo ${args.join(" ")}\n`);
  const run = spawnSync(file, argv, { stdio: "inherit", env: environment, windowsHide: true });
  if (run.status === 0) continue;
  failed++;
  if (install && run.status === 101) process.stderr.write(`(if cargo ${args[0]} isn't installed: ${install})\n`);
}
process.stderr.write(failed ? `\n${failed} of ${checks.length} checks failed.\n` : `\nAll ${checks.length} checks pass.\n`);
process.exitCode = failed ? 1 : 0;
