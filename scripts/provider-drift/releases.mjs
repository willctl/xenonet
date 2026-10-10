// Where each provider publishes its versions, and how to ask: npm's
// dist-tags, a GitHub repository's latest release, an APT repository's
// package index, a Mac app's Sparkle feed, or an update server's JSON. These are the calls a release watcher would make to see
// that something new is out (see registry.mjs). Each answers
// { versions: { <tag>: <version> }, url } or throws.
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { parseVersion, compareVersions } from "../../core/vendor.js";
import { cleanVersion } from "../../core/drift.js";

const UA = "xenonet-provider-drift (+https://github.com/willctl/xenonet)";
const TIMEOUT_MS = 20_000;

async function get(url, { headers = {}, fetchImpl = fetch } = {}) {
  const res = await fetchImpl(url, { headers: { "user-agent": UA, ...headers }, signal: AbortSignal.timeout(TIMEOUT_MS) });
  if (!res.ok) throw new Error(`${url} answered ${res.status}`);
  return res;
}

// npm's dist-tags for a package: { latest, next, stable, … }, only the tags
// whose value reads as a version.
export async function npmDistTags(pkg, opts) {
  const url = `https://registry.npmjs.org/-/package/${pkg.replace("/", "%2f")}/dist-tags`;
  const tags = await (await get(url, opts)).json();
  return { url, versions: Object.fromEntries(Object.entries(tags).filter(([, v]) => cleanVersion(v) && parseVersion(v))) };
}

// The version a GitHub release says: from its name (`pattern` picks it
// out, for a project whose tags are dates) or its tag.
export function releaseVersion(release, pattern = null) {
  const from = pattern ? new RegExp(pattern).exec(release?.name ?? "")?.[1] : String(release?.tag_name ?? "").replace(/^v/, "");
  return from && cleanVersion(from) && parseVersion(from) ? from : null;
}

// A repository's latest release (not a draft or prerelease).
export async function githubLatest(repo, { pattern = null, token = process.env.GH_TOKEN ?? process.env.GITHUB_TOKEN, ...opts } = {}) {
  const url = `https://api.github.com/repos/${repo}/releases/latest`;
  const release = await (await get(url, { ...opts, headers: { accept: "application/vnd.github+json", ...(token ? { authorization: `Bearer ${token}` } : {}) } })).json();
  const version = releaseVersion(release, pattern);
  if (!version) throw new Error(`${repo}'s latest release (${release?.tag_name ?? "?"}) doesn't say a version`);
  return { url: release.html_url ?? url, versions: { latest: version } };
}

// The newest version of `pkg` in an APT package index (a Packages file).
export function newestInPackages(text, pkg) {
  let newest = null;
  for (const stanza of String(text).split(/\n\s*\n/)) {
    if (!new RegExp(`^Package:\\s*${pkg.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\s*$`, "m").test(stanza)) continue;
    const v = /^Version:\s*(\S+)\s*$/m.exec(stanza)?.[1];
    const parsed = v && cleanVersion(v) && parseVersion(v);
    if (parsed && (!newest || compareVersions(parsed, newest[1]) > 0)) newest = [v, parsed];
  }
  return newest?.[0] ?? null;
}

export async function aptLatest({ index, pkg }, opts) {
  const version = newestInPackages(await (await get(index, opts)).text(), pkg);
  if (!version) throw new Error(`${index} lists no ${pkg}`);
  return { url: index, versions: { latest: version } };
}

// The newest version in a Sparkle appcast, the update feed a Mac app's
// Info.plist names (SUFeedURL): each item's sparkle:shortVersionString, as
// an element or, in older feeds, an attribute of its enclosure. That's the
// version the app shows, its CFBundleShortVersionString.
export function newestInAppcast(text) {
  let newest = null;
  for (const item of String(text).match(/<item[\s>][\s\S]*?<\/item>/g) ?? []) {
    const v = /<sparkle:shortVersionString>\s*([^<\s]+)\s*<\/sparkle:shortVersionString>/.exec(item)?.[1] ?? /\bsparkle:shortVersionString\s*=\s*"([^"]+)"/.exec(item)?.[1];
    const parsed = v && cleanVersion(v) && parseVersion(v);
    if (parsed && (!newest || compareVersions(parsed, newest[1]) > 0)) newest = [v, parsed];
  }
  return newest?.[0] ?? null;
}

export async function sparkleLatest(feed, opts) {
  const version = newestInAppcast(await (await get(feed, opts)).text());
  if (!version) throw new Error(`${feed} lists no version`);
  return { url: feed, versions: { latest: version } };
}

// The version an update server's JSON answer names, at `field` (a key,
// or keys joined with dots), when it reads as a version.
export function versionInJson(body, field = "version") {
  const v = String(field)
    .split(".")
    .reduce((o, k) => (o != null && typeof o === "object" && Object.hasOwn(o, k) ? o[k] : null), body);
  return typeof v === "string" && cleanVersion(v) && parseVersion(v) ? v : null;
}

export async function jsonLatest({ url, field }, opts) {
  const version = versionInJson(await (await get(url, opts)).json(), field);
  if (!version) throw new Error(`${url} doesn't say a version`);
  return { url, versions: { latest: version } };
}

// The versions a provider's release descriptor (registry.mjs) points at.
export async function latestVersions(release, opts = {}) {
  if (!release) return null;
  if (release.npm) return npmDistTags(release.npm, opts);
  if (release.github) return githubLatest(release.github, { pattern: release.pattern, ...opts });
  if (release.apt) return aptLatest(release.apt, opts);
  if (release.sparkle) return sparkleLatest(release.sparkle, opts);
  if (release.json) return jsonLatest(release.json, opts);
  return null;
}

// ---------- installing ----------

const SAFE = /^[@a-z0-9][\w./-]*$/i;
const SAFE_VERSION = /^[0-9A-Za-z.+-]+$/;

// How to run npm without a shell: its CLI script under this Node (npm run
// says where; Node's installers put it beside node), or npm on PATH.
function npmCommand() {
  const candidates = [process.env.npm_execpath, path.join(path.dirname(process.execPath), "node_modules", "npm", "bin", "npm-cli.js"), path.join(path.dirname(process.execPath), "..", "lib", "node_modules", "npm", "bin", "npm-cli.js")];
  const cli = candidates.find((f) => f && /npm-cli\.js$/.test(f) && fs.existsSync(f));
  return cli ? [process.execPath, [cli]] : ["npm", []];
}

// Installs `pkg@version` into a folder of its own under `work` (made
// afresh), without running any package's install scripts, and returns the
// folder its node_modules is in.
export async function npmInstall(pkg, version, work) {
  if (!SAFE.test(pkg) || !SAFE_VERSION.test(version)) throw new Error(`not a package to install: ${pkg}@${version}`);
  const dir = path.join(work, `${pkg.replace(/[@/]/g, "_")}-${version}`);
  fs.rmSync(dir, { recursive: true, force: true });
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, "package.json"), JSON.stringify({ private: true, name: "xenonet-provider-drift-install" }) + "\n");
  const args = ["install", "--no-save", "--no-package-lock", "--no-audit", "--no-fund", "--ignore-scripts", "--loglevel=error", `${pkg}@${version}`];
  const [cmd, pre] = npmCommand();
  const result = await new Promise((resolve) => {
    let err = "";
    const child = spawn(cmd, [...pre, ...args], { cwd: dir, stdio: ["ignore", "ignore", "pipe"], windowsHide: true });
    child.stderr.setEncoding("utf8").on("data", (c) => (err = (err + c).slice(-2000)));
    child.on("error", (e) => resolve({ code: null, err: e.message }));
    child.on("close", (c) => resolve({ code: c, err }));
  });
  if (result.code !== 0) throw new Error(`npm install ${pkg}@${version} failed (${result.code}): ${result.err.trim().split(/\r?\n/).slice(-3).join(" | ")}`);
  return dir;
}

// A package's own package.json, in an install from npmInstall().
export const installedPackage = (dir, pkg) => JSON.parse(fs.readFileSync(path.join(dir, "node_modules", ...pkg.split("/"), "package.json"), "utf8"));
