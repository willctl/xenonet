import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createHash } from "node:crypto";
import yaml from "js-yaml";
import { checkVersion, verifyArtifacts, signingEnvironment } from "../scripts/release.mjs";

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
