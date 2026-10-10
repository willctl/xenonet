// CI and local release builds use the same production package settings.
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { signingEnvironment } from "./release.mjs";

const pkg = JSON.parse(fs.readFileSync("package.json", "utf8"));
const platform = process.platform;
if (!["win32", "darwin"].includes(platform)) throw new Error("Build installers on Windows or macOS.");
const signed = process.env.XENONET_SIGN === "1";
const config = structuredClone(pkg.build);
config.directories.output = process.env.XENONET_RELEASE_DIR || config.directories.output;
config.extraMetadata = { xenonetMacAutoUpdate: platform === "darwin" && signed };
if (signed) {
  if (!process.env.CSC_LINK) throw new Error("A signing certificate is required when XENONET_SIGN=1.");
  config.forceCodeSigning = true;
}
if (platform === "darwin") {
  // ZIP is the updater payload; DMG is the human download. A PKG would
  // require a second certificate and offers no benefit to the updater.
  config.mac.target = ["dmg", "zip"];
  if (signed) {
    for (const name of ["APPLE_ID", "APPLE_APP_SPECIFIC_PASSWORD", "APPLE_TEAM_ID"]) {
      if (!process.env[name]) throw new Error(`Missing ${name} for notarization.`);
    }
    delete config.mac.identity;
    config.mac.hardenedRuntime = true;
    config.mac.notarize = true;
  }
}
fs.mkdirSync(".xenonet", { recursive: true });
fs.writeFileSync(".xenonet/release-builder.json", JSON.stringify(config, null, 2));
function run(command, args) {
  const result = spawnSync(command, args, { stdio: "inherit", shell: platform === "win32", env: signingEnvironment(process.env, signed) });
  if (result.status !== 0) process.exit(result.status ?? 1);
}
run("npm", ["run", "build"]);
run("npm", ["run", "icon"]);
for (const arch of platform === "win32" ? ["x64"] : ["arm64", "x64"]) {
  // The first-install bundle includes all three components. Desktop's
  // updater payload contains only Desktop; existing services keep running.
  for (const bundled of [true, false]) {
    const build = structuredClone(config);
    if (platform === "win32") build.nsis.artifactName = bundled ? "XenoNet-Setup-${version}.${ext}" : "XenoNet-Desktop-Setup-${version}.${ext}";
    else build.mac.target = bundled ? ["dmg"] : ["zip"];
    fs.writeFileSync(".xenonet/release-builder.json", JSON.stringify(build, null, 2));
    process.env.XENONET_BUNDLE = bundled ? "1" : "0";
    run("npx", ["electron-builder", "--config", ".xenonet/release-builder.json", platform === "win32" ? "--win" : "--mac", `--${arch}`, "--publish", "never"]);
  }
  if (platform === "darwin") fs.renameSync(path.join(config.directories.output, "latest-mac.yml"), path.join(config.directories.output, `mac-${arch}.yml`));
}
