// What a release ships: the payload folder that `xenolith install` installs from (xenolith and
// xenolith-tray, the window's files in ui/, the tray icons in icons/), and the release archives that
// carry it. Node is needed here only to build the window and the icons.
//
//   node scripts/package.mjs [--out <dir>] [--bin <dir with the Rust programs>] [--no-cargo]
//     The payload in one folder (default dist/package), as `npm run install-local` installs it.
//     The programs come from `cargo build --release --workspace` (when cargo is on PATH or in
//     %LOCALAPPDATA%\cargo, unless --no-cargo) or, without cargo, from rust/target/release as they are.
//
//   node scripts/package.mjs --target <darwin-arm64|darwin-x64|linux-x64|win32-x64> [--out <dir>] [--verify]
//     One release archive in dist/release (or --out): xenolith-<version>-<target>.tar.gz, or .zip
//     for Windows, holding the folder xenolith-<version>-<target>/ with the payload, LICENSE,
//     README.md and SECURITY.md, and SHA256SUMS over the archives in that folder. The programs are
//     built with Cargo's dist profile and with no path from this machine in them (the home folder,
//     rustup, cargo's registry and the checkout remapped), stripped with Xcode's strip and ad hoc
//     signed on macOS, then checked for the home folder's path. macOS builds both CPUs on a Mac;
//     Linux and Windows build their own. --verify then runs scripts/verify-archive.mjs on it.
//
//   node scripts/package.mjs --sums [<dir>]
//     SHA256SUMS over the release archives in <dir> (default dist/release).

import { execFileSync, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { entriesOf, tarGz, writeSums, zip } from './archive.mjs';
import { buildIcons } from './build-icons.mjs';

const REPO = path.resolve(fileURLToPath(new URL('..', import.meta.url)));
const EXE = process.platform === 'win32' ? '.exe' : '';

export const TARGETS = {
  'darwin-arm64': 'aarch64-apple-darwin',
  'darwin-x64': 'x86_64-apple-darwin',
  'linux-x64': 'x86_64-unknown-linux-gnu',
  'win32-x64': 'x86_64-pc-windows-msvc',
};

export const version = () => JSON.parse(fs.readFileSync(path.join(REPO, 'package.json'), 'utf8')).version;

function findCargo() {
  const r = spawnSync(process.platform === 'win32' ? 'where.exe' : 'which', ['cargo'], { encoding: 'utf8' });
  if (r.status === 0) return r.stdout.split(/\r?\n/)[0].trim();
  const local = path.join(process.env.LOCALAPPDATA || '', 'cargo', 'bin', `cargo${EXE}`);
  return process.env.LOCALAPPDATA && fs.existsSync(local) ? local : null;
}

/** The window (with the version check in scripts/build.mjs) and the tray icons, into dist/. */
function buildWeb() {
  const build = spawnSync(process.execPath, [path.join(REPO, 'scripts', 'build.mjs')], { cwd: REPO, encoding: 'utf8' });
  if (build.status !== 0) throw new Error(`build failed:\n${build.stdout}${build.stderr}`);
  buildIcons({ preview: false });
}

/** Copies the programs in `from` and the built window and icons into a fresh folder `out`. */
function assemble(from, out, exe) {
  const programs = [`xenolith${exe}`, `xenolith-tray${exe}`];
  for (const p of programs) if (!fs.existsSync(path.join(from, p))) throw new Error(`${p} not found in ${from}: build it (cargo build --release --workspace in rust/) or pass --bin`);
  fs.rmSync(out, { recursive: true, force: true });
  fs.mkdirSync(out, { recursive: true });
  // A GNU cross build loads WebView2 from a DLL beside the tray (the MSVC build links it in).
  for (const f of [...programs, 'WebView2Loader.dll']) if (fs.existsSync(path.join(from, f))) fs.copyFileSync(path.join(from, f), path.join(out, f));
  fs.cpSync(path.join(REPO, 'dist', 'runtime', 'ui'), path.join(out, 'ui'), { recursive: true });
  fs.mkdirSync(path.join(out, 'icons'));
  for (const f of fs.readdirSync(path.join(REPO, 'dist', 'icons'))) if (/^xenolith-[a-z]+(-256)?\.(ico|png|svg)$/.test(f)) fs.copyFileSync(path.join(REPO, 'dist', 'icons', f), path.join(out, 'icons', f));
}

/** Builds and assembles the payload. Returns {dir, built} or throws. */
export function packagePayload({ out = path.join(REPO, 'dist', 'package'), bin = null, cargo = true, log = console.log } = {}) {
  buildWeb();
  let built = false;
  const c = !bin && cargo ? findCargo() : null;
  if (c) {
    log(`cargo build --release --workspace (${c})`);
    const r = spawnSync(c, ['build', '--release', '--workspace', '--locked'], { cwd: path.join(REPO, 'rust'), stdio: 'inherit' });
    if (r.status !== 0) throw new Error('cargo build --release failed');
    built = true;
  }
  const from = bin || path.join(REPO, 'rust', 'target', 'release');
  assemble(from, out, EXE);
  return { dir: out, built, from };
}

// ---------------------------------------------------------------- release archives

const slashes = (p) => [...new Set([p, p.replace(/\\/g, '/')])];

/**
 * rustc flags that take this machine's folders out of what it writes. When several prefixes match,
 * rustc applies the last, so the most specific come last.
 */
export function remapFlags({ home = os.homedir(), env = process.env, checkout = REPO } = {}) {
  const cargoHome = env.CARGO_HOME || path.join(home, '.cargo');
  const pairs = [
    [home, '~'],
    [env.RUSTUP_HOME || path.join(home, '.rustup'), '/rustup'],
    [path.join(cargoHome, 'git'), '/cargo/git'],
    [path.join(cargoHome, 'registry'), '/cargo/registry'],
    [checkout, '/xenolith'],
  ];
  return pairs.flatMap(([from, to]) => slashes(from).map((f) => `--remap-path-prefix=${f}=${to}`));
}

/** Throws when a file under `dir` holds the home folder's or the checkout's path, in any form it is written. */
export function checkNoLocalPaths(dir, { home = os.homedir(), checkout = REPO, platform = process.platform } = {}) {
  const fold = platform === 'win32';
  const needles = [...new Set([home, checkout].flatMap((p) => [...slashes(p), p.replace(/\\/g, '\\\\')]))].filter((p) => p.length > 3);
  const forms = needles.flatMap((n) => {
    const s = fold ? n.toLowerCase() : n;
    return [Buffer.from(s, 'utf8'), Buffer.from(s, 'utf16le')];
  });
  const walk = (d) => fs.readdirSync(d, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? walk(path.join(d, e.name)) : [path.join(d, e.name)]));
  for (const f of walk(dir)) {
    let bytes = fs.readFileSync(f);
    // Windows paths are written in either case; ASCII folding is enough for the comparison.
    if (fold) bytes = Buffer.from(bytes.toString('latin1').toLowerCase(), 'latin1');
    if (forms.some((n) => bytes.includes(n))) {
      throw new Error(`${path.relative(dir, f)} contains this machine's home folder or checkout path; build it with scripts/package.mjs --target, which remaps them`);
    }
  }
}

/** Builds the programs for one release target; returns the folder they are in. */
function buildRelease(target, log) {
  const triple = TARGETS[target];
  const [platform, arch] = target.split('-');
  if (platform !== process.platform || (platform !== 'darwin' && arch !== process.arch)) {
    throw new Error(`${target} builds on ${platform === 'darwin' ? 'a Mac' : `${platform} ${arch}`} (this is ${process.platform} ${process.arch})`);
  }
  const cargo = findCargo();
  if (!cargo) throw new Error('cargo not found: install rustup (the toolchain is the one rust/rust-toolchain.toml pins)');
  // CARGO_ENCODED_RUSTFLAGS keeps a path with spaces in one flag; RUSTFLAGS would split it.
  const flags = [...(process.env.CARGO_ENCODED_RUSTFLAGS ? process.env.CARGO_ENCODED_RUSTFLAGS.split('\x1f') : (process.env.RUSTFLAGS || '').split(/\s+/)), ...remapFlags()].filter(Boolean);
  const env = { ...process.env, CARGO_ENCODED_RUSTFLAGS: flags.join('\x1f') };
  delete env.RUSTFLAGS;
  log(`cargo build --profile dist --workspace --target ${triple}`);
  const r = spawnSync(cargo, ['build', '--profile', 'dist', '--workspace', '--locked', '--target', triple], { cwd: path.join(REPO, 'rust'), stdio: 'inherit', env });
  if (r.status !== 0) throw new Error(`cargo build for ${triple} failed (rustup target add ${triple}, if it is missing)`);
  const dir = path.join(REPO, 'rust', 'target', triple, 'dist');
  if (platform === 'darwin') {
    for (const p of ['xenolith', 'xenolith-tray']) {
      const file = path.join(dir, p);
      // The dist profile strips with the toolchain's rust-objcopy, which can't load its LLVM library
      // on GitHub's Mac runners: rustc only warns and leaves the symbol table, which names the build
      // machine's folders. Xcode's strip takes it out whatever happened, before the signature.
      execFileSync('xcrun', ['strip', file], { stdio: 'inherit' });
      // Ad hoc, as XenoNet's are: a certificate carries its owner's name.
      execFileSync('codesign', ['--sign', '-', '--force', file], { stdio: 'inherit' });
    }
  }
  return dir;
}

/** Builds one target's archive into `outDir` and refreshes SHA256SUMS there. Returns the archive's path. */
export function packageRelease(target, { outDir = path.join(REPO, 'dist', 'release'), log = console.log } = {}) {
  if (!TARGETS[target]) throw new Error(`unknown target ${target} (one of ${Object.keys(TARGETS).join(', ')})`);
  buildWeb();
  const bin = buildRelease(target, log);
  const name = `xenolith-${version()}-${target}`;
  const win = target.startsWith('win32');
  const stage = path.join(outDir, name);
  assemble(bin, stage, win ? '.exe' : '');
  for (const f of ['LICENSE', 'README.md', 'SECURITY.md']) fs.copyFileSync(path.join(REPO, f), path.join(stage, f));
  checkNoLocalPaths(stage);
  const programs = new Set(['xenolith', 'xenolith-tray', 'xenolith.exe', 'xenolith-tray.exe', 'WebView2Loader.dll'].map((p) => `${name}/${p}`));
  const entries = entriesOf(stage, (rel) => programs.has(rel));
  const file = path.join(outDir, `${name}.${win ? 'zip' : 'tar.gz'}`);
  fs.writeFileSync(file, win ? zip(entries) : tarGz(entries));
  fs.rmSync(stage, { recursive: true, force: true });
  writeSums(outDir);
  log(`${path.relative(process.cwd(), file)} (${(fs.statSync(file).size / 1048576).toFixed(1)} MB, ${entries.length} entries)`);
  return file;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  const opt = (n) => {
    const i = args.indexOf(n);
    return i >= 0 ? args[i + 1] : undefined;
  };
  try {
    if (args.includes('--sums')) {
      const dir = path.resolve(opt('--sums') && !opt('--sums').startsWith('--') ? opt('--sums') : path.join(REPO, 'dist', 'release'));
      console.log(`SHA256SUMS -> ${writeSums(dir).join(', ')}`);
    } else if (opt('--target')) {
      const file = packageRelease(opt('--target'), { outDir: opt('--out') ? path.resolve(opt('--out')) : undefined });
      if (args.includes('--verify')) {
        const { verifyArchive } = await import('./verify-archive.mjs');
        await verifyArchive(file);
      }
    } else {
      const r = packagePayload({ out: opt('--out') ? path.resolve(opt('--out')) : undefined, bin: opt('--bin') || null, cargo: !args.includes('--no-cargo') });
      console.log(`package -> ${r.dir} (programs ${r.built ? 'built with cargo' : `from ${r.from}`})`);
    }
  } catch (e) {
    console.error(`package failed: ${e.message}`);
    process.exit(1);
  }
}
