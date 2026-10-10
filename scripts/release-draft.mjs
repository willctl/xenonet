import fs from "node:fs";
import { execFileSync, spawnSync } from "node:child_process";
import { checkVersion, verifyArtifacts } from "./release.mjs";

const pkg = JSON.parse(fs.readFileSync("package.json", "utf8"));
const tag = process.env.GITHUB_REF_NAME;
checkVersion(pkg, JSON.parse(fs.readFileSync("package-lock.json", "utf8")), tag);
const assets = verifyArtifacts("release", pkg.version, { components: true }).map((file) => `release/${file}`);
const existing = spawnSync("gh", ["release", "view", tag, "--json", "isDraft"], { encoding: "utf8" });
if (existing.status === 0) {
  if (!JSON.parse(existing.stdout).isDraft) throw new Error("This release is already public. Ship a new version instead of replacing its files.");
  execFileSync("gh", ["release", "upload", tag, ...assets, "--clobber"], { stdio: "inherit" });
} else {
  execFileSync("gh", ["release", "create", tag, ...assets, "--verify-tag", "--draft", "--title", `XenoNet ${pkg.version}`, "--notes-file", `docs/releases/${pkg.version}.md`], { stdio: "inherit" });
}
