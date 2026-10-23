// Checks a release archive the way a person uses it, in a fake home: unpack it with the system's tar
// into a temporary folder, run `xenolith install --sandbox` from the unpacked folder, check where the
// files landed and the installed version, start the installed server on a free port and ask it with
// `xenolith status`, then uninstall. Nothing outside the temporary folder is touched: the installer's
// --sandbox writes files only (no service, no sign-in entries, no app CLIs), and every program here
// runs with HOME, the XDG folders and AppData pointed into the fake home and no XENOLITH_* settings.
//
//   node scripts/verify-archive.mjs <xenolith-<version>-<target>.tar.gz|.zip> [--keep]
//
// The archive must be for this machine's OS and CPU.

import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const EXE = process.platform === 'win32' ? '.exe' : '';

/** The environment of every program the check starts: the fake home, and none of the caller's XENOLITH_* settings. */
function sandboxEnv(home) {
  const env = Object.fromEntries(Object.entries(process.env).filter(([k]) => !k.toUpperCase().startsWith('XENOLITH_')));
  const p = (rel) => path.join(home, rel);
  return {
    ...env,
    HOME: home,
    USERPROFILE: home,
    APPDATA: p('AppData/Roaming'),
    LOCALAPPDATA: p('AppData/Local'),
    XDG_CONFIG_HOME: p('.config'),
    XDG_DATA_HOME: p('.local/share'),
    XDG_STATE_HOME: p('.local/state'),
    XDG_CACHE_HOME: p('.cache'),
    CODEX_HOME: p('.codex'),
  };
}

function freePort() {
  return new Promise((resolve, reject) => {
    const s = net.createServer();
    s.once('error', reject);
    s.listen(0, '127.0.0.1', () => {
      const { port } = s.address();
      s.close(() => resolve(port));
    });
  });
}

function health(port) {
  return new Promise((resolve) => {
    const req = http.get({ host: '127.0.0.1', port, path: '/health', timeout: 2000 }, (res) => {
      let body = '';
      res.on('data', (c) => (body += c));
      res.on('end', () => {
        try {
          resolve(JSON.parse(body));
        } catch {
          resolve(null);
        }
      });
    });
    req.on('error', () => resolve(null));
    req.on('timeout', () => req.destroy());
  });
}

const walk = (d) => fs.readdirSync(d, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? walk(path.join(d, e.name)) : [path.join(d, e.name)]));
const inside = (child, parent) => !path.relative(parent, child).startsWith('..') && !path.isAbsolute(path.relative(parent, child));

export async function verifyArchive(file, { keep = false, log = (m) => console.log(`verify: ${m}`) } = {}) {
  const base = path.basename(file).replace(/\.(tar\.gz|zip)$/, '');
  const m = base.match(/^xenolith-(.+)-(darwin|linux|win32)-(arm64|x64)$/);
  if (!m) throw new Error(`${path.basename(file)} is not named xenolith-<version>-<platform>-<arch>`);
  const [, version, platform, arch] = m;
  if (platform !== process.platform || arch !== process.arch) throw new Error(`${base} runs on ${platform} ${arch}, not on this ${process.platform} ${process.arch}`);
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'xenolith-archive-'));
  const unpacked = path.join(tmp, 'unpacked');
  const home = path.join(tmp, 'home');
  fs.mkdirSync(unpacked);
  fs.mkdirSync(home);
  const env = sandboxEnv(home);
  let server = null;
  try {
    // Windows' own tar (bsdtar) reads a zip; a Git or MSYS tar earlier on PATH would not.
    const tar = process.platform === 'win32' ? path.join(process.env.SystemRoot || 'C:\\Windows', 'System32', 'tar.exe') : 'tar';
    const x = spawnSync(tar, ['-xf', path.resolve(file), '-C', unpacked], { encoding: 'utf8' });
    assert.equal(x.status, 0, `tar could not unpack ${file}: ${x.stderr}`);
    assert.deepEqual(fs.readdirSync(unpacked), [base], 'one folder at the top, named for the archive');
    const dir = path.join(unpacked, base);
    for (const f of [`xenolith${EXE}`, `xenolith-tray${EXE}`, 'ui/index.html', 'icons/xenolith-healthy.ico', 'LICENSE', 'README.md']) assert.ok(fs.existsSync(path.join(dir, f)), `${f} is in the archive`);
    if (process.platform !== 'win32') for (const f of ['xenolith', 'xenolith-tray']) assert.ok(fs.statSync(path.join(dir, f)).mode & 0o100, `${f} is executable`);
    log(`unpacked into ${unpacked}`);

    // Run as a person would, from the unpacked folder: `./xenolith ...`.
    const own = process.platform === 'win32' ? path.join(dir, `xenolith${EXE}`) : `./xenolith`;
    const run = (cmd, args, opts = {}) => spawnSync(cmd, args, { cwd: dir, env, encoding: 'utf8', timeout: 180_000, ...opts });
    const v = run(own, ['--version']);
    assert.equal(v.status, 0, v.stderr);
    assert.match(v.stdout, new RegExp(`\\b${version.replace(/\./g, '\\.')}\\b`), `--version prints ${version}`);
    log(`./xenolith --version: ${v.stdout.trim()}`);

    const inst = run(own, ['install', '--sandbox', home]);
    assert.equal(inst.status, 0, `install failed:\n${inst.stdout}${inst.stderr}`);
    assert.match(inst.stdout, /MCP over stdio OK/, inst.stdout);
    assert.match(inst.stdout, /hook OK under/, inst.stdout);
    const runtime = inst.stdout.match(/^ {2}Runtime: +(.+)$/m)?.[1]?.trim();
    assert.ok(runtime, `the installer names the runtime folder:\n${inst.stdout}`);
    const agent = path.join(runtime, `xenolith${EXE}`);
    const tray = walk(home).find((f) => path.basename(f) === `xenolith-tray${EXE}`);
    for (const [what, f] of [['xenolith', agent], ['the window', path.join(runtime, 'ui', 'index.html')], ['xenolith-tray', tray]]) {
      assert.ok(f && fs.existsSync(f), `${what} installed`);
      assert.ok(inside(path.resolve(f), home), `${what} landed in the fake home (${f})`);
    }
    log(`installed into the fake home: ${path.relative(home, agent)}, ${path.relative(home, tray)}`);
    const iv = run(agent, ['--version']);
    assert.match(iv.stdout, new RegExp(`\\b${version.replace(/\./g, '\\.')}\\b`), `the installed xenolith --version prints ${version}`);
    log(`installed xenolith --version: ${iv.stdout.trim()}`);

    // The installed server, as the service would run it, on a port the OS says is free. It stops
    // when this script does (--parent-stdin), however the script ends.
    const port = await freePort();
    server = spawn(agent, ['serve', '--http', '--port', String(port), '--parent-stdin'], { cwd: home, env, stdio: ['pipe', 'ignore', 'pipe'] });
    let serverErr = '';
    server.stderr.on('data', (c) => (serverErr += c));
    let h = null;
    for (let i = 0; i < 100 && !(h && h.ok); i++) {
      h = await health(port);
      if (!(h && h.ok)) await new Promise((r) => setTimeout(r, 200));
    }
    assert.ok(h && h.ok, `the server answered /health on ${port}: ${JSON.stringify(h)}\n${serverErr}`);
    const st = run(agent, ['status', '--port', String(port), '--json']);
    const status = JSON.parse(st.stdout || 'null');
    assert.equal(st.status, 0, `xenolith status exits 0:\n${st.stdout}${st.stderr}`);
    assert.equal(status?.service?.health, 'ok', `xenolith status sees a healthy service:\n${st.stdout}`);
    log(`xenolith status --port ${port}: service ${status.service.health}, v${status.service.version ?? '?'}, audit chain ${status.audit.ok ? 'ok' : 'not ok'}`);
    server.kill();
    await new Promise((r) => (server.exitCode !== null ? r() : server.once('exit', r)));
    server = null;

    const un = run(own, ['uninstall', '--sandbox', home]);
    assert.equal(un.status, 0, `uninstall failed:\n${un.stdout}${un.stderr}`);
    assert.ok(!fs.existsSync(tray), 'uninstall removed the tray');
    assert.ok(!walk(home).some((f) => f.includes(`${path.sep}marketplace${path.sep}`)), 'uninstall removed the plugin marketplace');
    log('uninstalled from the fake home');
    log(`${base}: ok`);
  } finally {
    if (server) server.kill();
    if (keep) log(`kept ${tmp}`);
    else fs.rmSync(tmp, { recursive: true, force: true, maxRetries: 5, retryDelay: 500 });
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  const file = args.find((a) => !a.startsWith('--'));
  if (!file) {
    console.error('usage: node scripts/verify-archive.mjs <archive> [--keep]');
    process.exit(2);
  }
  try {
    await verifyArchive(file, { keep: args.includes('--keep') });
  } catch (e) {
    console.error(`verify failed: ${e.message}`);
    process.exit(1);
  }
}
