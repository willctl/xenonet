// electron-builder hooks (see "build" in package.json).
//
// afterPack: refuses to ship files that contain the builder's home folder
//   path, then builds the components (the xenonet binary's packages) beside
//   the app.
// afterSign, afterAllArtifactBuild: the macOS app is ad-hoc signed and the
//   package unsigned, so no certificate (which carries a personal name) ends
//   up in a release by accident. Set XENONET_SIGN=1 to sign on purpose.
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import { buildComponents } from "./component-build.mjs";
import os from "node:os";
import path from "node:path";

const ARCH = { 0: "ia32", 1: "x64", 2: "armv7l", 3: "arm64", 4: "universal" };

function appDir(context) {
  return path.join(context.packager.getResourcesDir(context.appOutDir), "app");
}

// Bundled text that mentions the home folder would publish the builder's
// user name (source maps from a local build, say).
function noHomePaths(context) {
  const home = os.homedir();
  const forms = [...new Set([home, home.replace(/\\/g, "/"), home.replace(/\\/g, "\\\\")])];
  const hits = [];
  const walk = (dir) => {
    for (const ent of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, ent.name);
      if (ent.isDirectory()) walk(full);
      else if (/\.(js|cjs|mjs|json|html|css|map|txt|md)$/i.test(ent.name) && fs.statSync(full).size < 8 << 20) {
        const text = fs.readFileSync(full, "utf8");
        if (forms.some((f) => text.includes(f))) hits.push(path.relative(appDir(context), full));
      }
    }
  };
  walk(appDir(context));
  if (hits.length) throw new Error(`these packaged files contain your home folder path:\n  ${hits.slice(0, 20).join("\n  ")}`);
}

export async function afterPack(context) {
  noHomePaths(context);
  const app = appDir(context);
  if (process.env.XENONET_BUNDLE !== "0") await buildComponents({ app, platform: context.electronPlatformName, arch: ARCH[context.arch], output: context.outDir, bundle: path.join(path.dirname(app), "components") });
}

export async function afterSign(context) {
  if (context.electronPlatformName !== "darwin" || process.env.XENONET_SIGN === "1") return;
  const app = path.join(context.appOutDir, `${context.packager.appInfo.productFilename}.app`);
  const info = execFileSync("sh", ["-c", 'codesign -dvv "$1" 2>&1 || true', "sh", app], { encoding: "utf8" });
  if (/^Authority=/m.test(info) || !/Signature=adhoc/.test(info)) {
    throw new Error("XenoNet.app was signed with a certificate. Release builds are ad-hoc signed so no personal name ends up in them; set XENONET_SIGN=1 to sign on purpose.");
  }
}

export async function afterAllArtifactBuild(result) {
  if (process.env.XENONET_SIGN === "1") return [];
  for (const file of result.artifactPaths.filter((f) => f.endsWith(".pkg"))) {
    const out = execFileSync("sh", ["-c", 'pkgutil --check-signature "$1" 2>&1 || true', "sh", file], { encoding: "utf8" });
    if (!/no signature/i.test(out)) throw new Error(`${path.basename(file)} is signed. Release packages are unsigned so no personal name ends up in them; set XENONET_SIGN=1 to sign on purpose.`);
  }
  return [];
}
