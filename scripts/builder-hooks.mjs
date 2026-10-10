// electron-builder hooks (see "build" in package.json).
//
// afterPack: the terminal's native module comes as one npm package per
//   platform and CPU (@lydell/node-pty-darwin-arm64 and so on), and npm only
//   installs the one for the machine doing the build. Each packaged app gets
//   exactly the one it runs on, fetched from the registry when it's missing
//   (an Intel build on an Apple silicon Mac, say). It also refuses to ship
//   files that contain the builder's home folder path.
// afterSign, afterAllArtifactBuild: the macOS app is ad-hoc signed and the
//   package unsigned, so no certificate (which carries a personal name) ends
//   up in a release by accident. Set XENONET_SIGN=1 to sign on purpose.
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import { buildComponents } from "./component-build.mjs";
import os from "node:os";
import path from "node:path";

const ARCH = { 0: "ia32", 1: "x64", 2: "armv7l", 3: "arm64", 4: "universal" };
const PTY = "@lydell/node-pty";

function appDir(context) {
  return path.join(context.packager.getResourcesDir(context.appOutDir), "app");
}

function fetchPackage(name, version, into) {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-pty-"));
  try {
    const tgz = execFileSync("npm", ["pack", `${name}@${version}`, "--silent", "--pack-destination", tmp], { encoding: "utf8", shell: process.platform === "win32" }).trim().split(/\r?\n/).pop();
    execFileSync("tar", ["-xzf", path.join(tmp, tgz), "-C", tmp]);
    fs.mkdirSync(path.dirname(into), { recursive: true });
    fs.cpSync(path.join(tmp, "package"), into, { recursive: true });
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
}

function nativePty(context) {
  const platform = context.electronPlatformName;
  const arch = ARCH[context.arch];
  const scope = path.join(appDir(context), "node_modules", "@lydell");
  const want = `node-pty-${platform}-${arch}`;
  for (const dir of fs.readdirSync(scope)) {
    if (dir.startsWith("node-pty-") && dir !== want) fs.rmSync(path.join(scope, dir), { recursive: true, force: true });
  }
  const target = path.join(scope, want);
  if (!fs.existsSync(target)) {
    const version = JSON.parse(fs.readFileSync(path.join(scope, "node-pty", "package.json"), "utf8")).optionalDependencies?.[`${PTY}-${platform}-${arch}`];
    if (!version) throw new Error(`${PTY} has no prebuilt package for ${platform}-${arch}`);
    console.log(`  • fetching ${PTY}-${platform}-${arch}@${version} for this build`);
    fetchPackage(`${PTY}-${platform}-${arch}`, version, target);
  }
  if (platform === "darwin") {
    const bin = path.join(target, "prebuilds", `${platform}-${arch}`);
    fs.chmodSync(path.join(bin, "spawn-helper"), 0o755);
    for (const file of ["pty.node", "spawn-helper"]) {
      const archs = execFileSync("lipo", ["-archs", path.join(bin, file)], { encoding: "utf8" }).trim().split(/\s+/);
      if (!archs.includes(arch === "x64" ? "x86_64" : arch)) throw new Error(`${file} in ${want} is built for ${archs.join(", ")}, not ${arch}`);
    }
  }
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
  nativePty(context);
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
