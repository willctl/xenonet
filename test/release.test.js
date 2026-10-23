import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";
import yaml from "js-yaml";
import { checkVersion, checkPlugin, pluginWithVersion, verifyArtifacts, signingEnvironment, PLUGIN_MANIFEST } from "../scripts/release.mjs";

test("unsigned builds ignore inherited signing credentials and empty secrets", () => {
  const env = { PATH: "tools", CSC_LINK: "certificate.p12", WIN_CSC_LINK: "", CSC_KEY_PASSWORD: "example", APPLE_ID: "release@example.test" };
  assert.deepEqual(signingEnvironment(env, false), { PATH: "tools", CSC_IDENTITY_AUTO_DISCOVERY: "false" });
  const signed = signingEnvironment(env, true);
  assert.equal(signed.CSC_LINK, "certificate.p12");
  assert.equal(signed.CSC_IDENTITY_AUTO_DISCOVERY, "true");
  assert.equal("WIN_CSC_LINK" in signed, false);
});

test("release tags, package and lockfile must agree", () => {
  const pkg = { version: "0.5.0" }, lock = { version: "0.5.0", packages: { "": pkg } };
  checkVersion(pkg, lock, "v0.5.0");
  assert.throws(() => checkVersion(pkg, lock, "main"));
  assert.throws(() => checkVersion(pkg, { ...lock, version: "0.4.0" }, "v0.5.0"));
  assert.throws(() => checkVersion({ version: "0.5.0-beta.1" }, lock, "v0.5.0-beta.1"));
});

test("XenoNet's plugin for Claude Code carries the package's version, and npm version sets it", () => {
  const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
  const read = (file) => JSON.parse(fs.readFileSync(path.join(repo, file), "utf8"));
  checkPlugin(read("package.json"), read(PLUGIN_MANIFEST));
  assert.equal(read("package.json").scripts.version, "node scripts/release.mjs plugin");
  assert.throws(() => checkPlugin({ version: "0.5.0" }, { version: "0.4.0" }), /npm run version sets it to 0\.5\.0/);
  const text = '{\n  "name": "xenonet",\n  "version": "0.4.0",\n  "keywords": ["a", "b"]\n}\n';
  assert.equal(pluginWithVersion(text, "0.5.0"), text.replace("0.4.0", "0.5.0"), "only the version changes");
  assert.throws(() => pluginWithVersion("{}", "0.5.0"));
});

test("release assembly checks hashes, requires both Mac CPUs and writes checksums", (t) => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-release-test-"));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  const file = (name) => {
    const bytes = Buffer.from(name);
    fs.writeFileSync(path.join(dir, name), bytes);
    return { url: name, size: bytes.length, sha512: createHash("sha512").update(bytes).digest("base64") };
  };
  const manifest = (name, files) => fs.writeFileSync(path.join(dir, name), yaml.dump({ version: "0.5.0", files }));
  manifest("latest.yml", [file("XenoNet-Setup-0.5.0.exe")]);
  file("XenoNet-Setup-0.5.0.exe.blockmap");
  for (const arch of ["arm64", "x64"]) {
    manifest(`mac-${arch}.yml`, [file(`XenoNet-0.5.0-${arch}.zip`), file(`XenoNet-0.5.0-${arch}.dmg`)]);
    file(`XenoNet-0.5.0-${arch}.zip.blockmap`);
  }
  const assets = verifyArtifacts(dir, "0.5.0");
  assert(assets.includes("latest-mac.yml"));
  assert.equal(yaml.load(fs.readFileSync(path.join(dir, "latest-mac.yml"), "utf8")).files.length, 4);
  assert.match(fs.readFileSync(path.join(dir, "SHA256SUMS.txt"), "utf8"), /^[a-f0-9]{64}  XenoNet-/);
  fs.appendFileSync(path.join(dir, "XenoNet-Setup-0.5.0.exe"), "tampered");
  assert.throws(() => verifyArtifacts(dir, "0.5.0"), /Hash or size mismatch/);
});

test("with components, each target's Service and Tools and the Mac's one universal tray go in components.json and the checksums", (t) => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-release-test-"));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  const file = (name) => {
    const bytes = Buffer.from(name);
    fs.writeFileSync(path.join(dir, name), bytes);
    return { url: name, size: bytes.length, sha512: createHash("sha512").update(bytes).digest("base64") };
  };
  const manifest = (name, files) => fs.writeFileSync(path.join(dir, name), yaml.dump({ version: "0.5.0", files }));
  manifest("latest.yml", [file("XenoNet-Desktop-Setup-0.5.0.exe")]);
  for (const name of ["XenoNet-Setup-0.5.0.exe", "XenoNet-Setup-0.5.0.exe.blockmap", "XenoNet-Desktop-Setup-0.5.0.exe.blockmap"]) file(name);
  for (const arch of ["arm64", "x64"]) {
    manifest(`mac-${arch}.yml`, [file(`XenoNet-0.5.0-${arch}.zip`), file(`XenoNet-0.5.0-${arch}.dmg`)]);
    file(`XenoNet-0.5.0-${arch}.zip.blockmap`);
  }
  const versions = { service: "0.5.0", tools: "0.5.0", tray: "0.4.1" };
  const part = (target, components) => {
    const [platform, arch] = target.split("-");
    const packages = components.map((component) => {
      const name = `XenoNet-${component}-${versions[component]}-${target}.tar.gz`;
      const bytes = Buffer.from(name);
      fs.writeFileSync(path.join(dir, name), bytes);
      return { format: 1, component, version: versions[component], platform, arch, file: name, sha256: createHash("sha256").update(bytes).digest("hex"), size: bytes.length };
    });
    fs.writeFileSync(path.join(dir, `components-${target}.json`), JSON.stringify({ format: 1, packages }));
  };
  for (const target of ["win32-x64", "darwin-arm64", "darwin-x64", "linux-x64"]) part(target, ["service", "tools"]);
  assert.throws(() => verifyArtifacts(dir, "0.5.0", { components: true, versions }), /ENOENT.*components-darwin-universal\.json/, "the tray is part of a release");
  part("darwin-universal", ["tray"]);
  const assets = verifyArtifacts(dir, "0.5.0", { components: true, versions });
  assert.ok(assets.includes("XenoNet-tray-0.4.1-darwin-universal.tar.gz"));
  const all = JSON.parse(fs.readFileSync(path.join(dir, "components.json"), "utf8")).packages;
  assert.deepEqual(all.filter((p) => p.component === "tray").map((p) => `${p.platform}-${p.arch}`), ["darwin-universal"]);
  // A client before the tray picks packages by its own platform and CPU,
  // which "universal" never is, so the tray's entry passes it by.
  assert.deepEqual([...new Set(all.filter((p) => p.platform === "darwin" && p.arch === "arm64").map((p) => p.component))], ["service", "tools"]);
  assert.match(fs.readFileSync(path.join(dir, "SHA256SUMS.txt"), "utf8"), /^[a-f0-9]{64}  XenoNet-tray-0\.4\.1-darwin-universal\.tar\.gz$/m);
  fs.appendFileSync(path.join(dir, "XenoNet-tray-0.4.1-darwin-universal.tar.gz"), "x");
  assert.throws(() => verifyArtifacts(dir, "0.5.0", { components: true, versions }), /Invalid component artifact: XenoNet-tray/);
});
