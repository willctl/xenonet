import fs from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
import { pathToFileURL } from "node:url";
import yaml from "js-yaml";

export function signingEnvironment(env, signed) {
  const result = { ...env, CSC_IDENTITY_AUTO_DISCOVERY: signed ? "true" : "false" };
  // An unset GitHub secret arrives as an empty string. The builder treats
  // an empty CSC_LINK as a file path, resolving it to the checkout itself.
  for (const key of ["CSC_LINK", "CSC_KEY_PASSWORD", "CSC_NAME", "CSC_KEYCHAIN", "WIN_CSC_LINK", "WIN_CSC_KEY_PASSWORD", "APPLE_ID", "APPLE_APP_SPECIFIC_PASSWORD", "APPLE_TEAM_ID"]) {
    if (!signed || !result[key]) delete result[key];
  }
  return result;
}

export function checkVersion(pkg, lock, tag) {
  if (!/^\d+\.\d+\.\d+$/.test(pkg.version)) throw new Error("Stable releases need a version such as 0.5.0.");
  if (tag !== `v${pkg.version}`) throw new Error(`Tag ${tag} must match v${pkg.version}.`);
  if (lock.version !== pkg.version || lock.packages[""].version !== pkg.version) throw new Error("package-lock.json has a different version.");
}

// XenoNet's plugin for Claude Code carries the package's version, so an
// update to it reaches Claude Code's plugin list with each release.
export const PLUGIN_MANIFEST = "plugins/xenonet/.claude-plugin/plugin.json";

export function checkPlugin(pkg, plugin) {
  if (plugin.version !== pkg.version) throw new Error(`${PLUGIN_MANIFEST} has version ${plugin.version}; npm run version sets it to ${pkg.version}.`);
}

// The manifest with the package's version, the rest of it as it was.
export function pluginWithVersion(text, version) {
  if (!/"version":\s*"[^"]*"/.test(text)) throw new Error(`${PLUGIN_MANIFEST} has no version to set.`);
  return text.replace(/("version":\s*")[^"]*(")/, `$1${version}$2`);
}

const digest = (file, algorithm, encoding) => createHash(algorithm).update(fs.readFileSync(file)).digest(encoding);

export function verifyArtifacts(dir, version, { components = false } = {}) {
  const expected = [`XenoNet-Setup-${version}.exe`, `XenoNet-Setup-${version}.exe.blockmap`, ...["arm64", "x64"].flatMap((arch) => [`XenoNet-${version}-${arch}.dmg`, `XenoNet-${version}-${arch}.zip`, `XenoNet-${version}-${arch}.zip.blockmap`])];
  if (components) expected.push(`XenoNet-Desktop-Setup-${version}.exe`, `XenoNet-Desktop-Setup-${version}.exe.blockmap`);
  for (const name of expected) {
    if (!fs.existsSync(path.join(dir, name)) || fs.statSync(path.join(dir, name)).size === 0) throw new Error(`Missing release file: ${name}`);
  }
  function readManifest(name) {
    const manifest = yaml.load(fs.readFileSync(path.join(dir, name), "utf8"));
    if (manifest.version !== version || !Array.isArray(manifest.files) || !manifest.files.length) throw new Error(`Wrong version or empty files in ${name}`);
    for (const file of manifest.files) {
      if (typeof file.url !== "string" || path.basename(file.url) !== file.url || !expected.includes(file.url)) throw new Error(`Unexpected file in ${name}`);
      const full = path.join(dir, file.url);
      if (digest(full, "sha512", "base64") !== file.sha512 || fs.statSync(full).size !== file.size) throw new Error(`Hash or size mismatch: ${file.url}`);
    }
    return manifest;
  }
  const windows = readManifest("latest.yml");
  if (!windows.files.some((file) => file.url.endsWith(".exe"))) throw new Error("Windows update has no installer.");
  const macs = ["arm64", "x64"].map((arch) => {
    const manifest = readManifest(`mac-${arch}.yml`);
    if (!manifest.files.some((file) => file.url === `XenoNet-${version}-${arch}.zip`)) throw new Error(`Mac ${arch} update has no ZIP.`);
    return manifest;
  });
  const merged = { ...macs[0], files: macs.flatMap((manifest) => manifest.files) };
  fs.writeFileSync(path.join(dir, "latest-mac.yml"), yaml.dump(merged));
  const componentAssets = [];
  if (components) {
    const packages = [];
    const versions = JSON.parse(fs.readFileSync("components/versions.json", "utf8"));
    for (const target of ["win32-x64", "darwin-arm64", "darwin-x64", "linux-x64"]) {
      const part = JSON.parse(fs.readFileSync(path.join(dir, `components-${target}.json`), "utf8"));
      if (part.format !== 1 || part.packages.length !== 2) throw new Error(`Incomplete components for ${target}.`);
      for (const component of ["service", "tools"]) {
        const item = part.packages.find((p) => p.component === component);
        const file = `XenoNet-${component}-${versions[component]}-${target}.tar.gz`;
        if (!item || item.file !== file || `${item.platform}-${item.arch}` !== target || item.version !== versions[component] || digest(path.join(dir, file), "sha256", "hex") !== item.sha256 || fs.statSync(path.join(dir, file)).size !== item.size) throw new Error(`Invalid component artifact: ${file}`);
        packages.push(item); componentAssets.push(file);
      }
    }
    fs.writeFileSync(path.join(dir, "components.json"), JSON.stringify({ format: 1, packages }, null, 2));
    componentAssets.push("components.json");
    if (!windows.files.some((f) => f.url === `XenoNet-Desktop-Setup-${version}.exe`)) throw new Error("Desktop updates must use the Desktop-only installer.");
  }
  const assets = [...componentAssets, ...expected, "latest.yml", "latest-mac.yml", ...fs.readdirSync(dir).filter((name) => /^XenoNet-.*\.dmg\.blockmap$/.test(name))].sort();
  fs.writeFileSync(path.join(dir, "SHA256SUMS.txt"), assets.map((name) => `${digest(path.join(dir, name), "sha256", "hex")}  ${name}\n`).join(""));
  return [...assets, "SHA256SUMS.txt"];
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const pkg = JSON.parse(fs.readFileSync("package.json", "utf8"));
  if (process.argv[2] === "check") {
    const tag = process.env.GITHUB_REF_TYPE === "tag" ? process.env.GITHUB_REF_NAME : `v${pkg.version}`;
    checkVersion(pkg, JSON.parse(fs.readFileSync("package-lock.json", "utf8")), tag);
    checkPlugin(pkg, JSON.parse(fs.readFileSync(PLUGIN_MANIFEST, "utf8")));
    const notes = fs.readFileSync(`docs/releases/${pkg.version}.md`, "utf8");
    if (!notes.trim() || /\b(TODO|TBD)\b/.test(notes)) throw new Error("Write release notes before tagging.");
    console.log(`Ready to build XenoNet ${pkg.version}.`);
  } else if (process.argv[2] === "plugin") {
    // npm version runs this once it has set the package's version.
    fs.writeFileSync(PLUGIN_MANIFEST, pluginWithVersion(fs.readFileSync(PLUGIN_MANIFEST, "utf8"), pkg.version));
    console.log(`${PLUGIN_MANIFEST}: ${pkg.version}`);
  } else if (process.argv[2] === "verify") {
    console.log(verifyArtifacts(process.argv[3] ?? "release", pkg.version, { components: true }).join("\n"));
  } else throw new Error("Use check, plugin or verify.");
}
