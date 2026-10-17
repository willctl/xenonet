// electron scripts/capture-window.mjs [--bin <xenolith>] [--out <png>] [--tmp <dir>]
//
// Captures the window for the showcase (docs/showcase/demo-page.png) from a throwaway instance:
// a temp wiki holding only the fictional docs/showcase/weekend-studio.md, served by the Rust
// program (default rust/target/release/xenolith) on a free port, with the window from
// dist/runtime/ui (run `npm run build` first). XENOLITH_HOME and XENOLITH_DIR always point at the
// temp folder, so the capture never reads a real wiki. The Electron window stays hidden and muted.

import { app, BrowserWindow, nativeTheme } from 'electron';
import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const args = process.argv.slice(2);
const opt = (n, d) => (args.includes(n) ? args[args.indexOf(n) + 1] : d);
const bin = path.resolve(opt('--bin', path.join(root, 'rust', 'target', 'release', process.platform === 'win32' ? 'xenolith.exe' : 'xenolith')));
const out = path.resolve(opt('--out', path.join(root, 'docs', 'showcase', 'demo-page.png')));
const tmpBase = path.resolve(opt('--tmp', os.tmpdir()));
const ui = path.join(root, 'dist', 'runtime', 'ui');
// The tray window's size, at 1x, so the renderer's crop lines up.
const WIDTH = 449;
const HEIGHT = 884;

const work = fs.mkdtempSync(path.join(tmpBase, 'xenolith-showcase-'));
const home = path.join(work, 'home');
const wiki = path.join(work, 'wiki');
fs.mkdirSync(path.join(wiki, 'pages'), { recursive: true });
fs.mkdirSync(home);
fs.writeFileSync(path.join(home, 'config.json'), JSON.stringify({ writeMode: 'curated' }));
fs.copyFileSync(path.join(root, 'docs', 'showcase', 'weekend-studio.md'), path.join(wiki, 'pages', 'weekend-studio.md'));

// Drop any inherited XENOLITH_* (a config or log dir override would point outside the temp folder).
const env = Object.fromEntries(Object.entries(process.env).filter(([k]) => !k.startsWith('XENOLITH_')));
Object.assign(env, { XENOLITH_HOME: home, XENOLITH_DIR: wiki, XENOLITH_UI_DIR: ui });

// The page shows its frontmatter date; in UTC it reads the same on every machine.
process.env.TZ = 'UTC';
app.commandLine.appendSwitch('force-device-scale-factor', '1');
// Electron writes its profile until after it quits, so it gets one reused folder of its own.
app.setPath('userData', path.join(tmpBase, 'xenolith-showcase-electron'));
app.dock?.hide();

let server;

function startServer() {
  const init = spawnSync(bin, ['serve', '--init'], { env, encoding: 'utf8' });
  if (init.status !== 0) throw new Error(`${bin} serve --init failed: ${init.error?.message || init.stderr || init.stdout}`);
  server = spawn(bin, ['serve', '--http', '--port', '0', '--parent-stdin'], { env, stdio: ['pipe', 'ignore', 'pipe'] });
  return new Promise((resolve, reject) => {
    let log = '';
    const timer = setTimeout(() => reject(new Error(`the server did not start:\n${log}`)), 15000);
    server.stderr.on('data', (d) => {
      log += d;
      const m = log.match(/listening on http:\/\/127\.0\.0\.1:([0-9]+)\//);
      if (m) {
        clearTimeout(timer);
        resolve(Number(m[1]));
      }
    });
    server.on('exit', (code) => reject(new Error(`the server exited (${code}):\n${log}`)));
  });
}

/** Stops the server (--parent-stdin: closing stdin is its stop signal; kill is the fallback), then removes the temp wiki. */
async function stop() {
  if (server && server.exitCode === null && server.signalCode === null) {
    const exited = new Promise((r) => server.once('exit', r));
    server.stdin.end();
    const timer = setTimeout(() => server.kill(), 3000);
    await exited;
    clearTimeout(timer);
  }
  fs.rmSync(work, { recursive: true, force: true });
}

async function capture(port) {
  nativeTheme.themeSource = 'dark';
  const win = new BrowserWindow({ width: WIDTH, height: HEIGHT, useContentSize: true, show: false, backgroundColor: '#15161b', webPreferences: { backgroundThrottling: false } });
  win.webContents.setAudioMuted(true);
  await win.loadURL(`http://127.0.0.1:${port}/ui/#/page/weekend-studio`);
  // Wait for the page and the service status to render, then for fonts and two frames.
  const ready = `(() => {
    const page = [...document.querySelectorAll('h1')].some((e) => e.textContent.includes('Weekend studio'));
    const pill = document.querySelector('header .pill');
    return page && !!pill && !pill.classList.contains('pill-wait');
  })()`;
  for (let i = 0; !(await win.webContents.executeJavaScript(ready)); i++) {
    if (i > 100) throw new Error('the page did not render');
    await new Promise((r) => setTimeout(r, 100));
  }
  await win.webContents.executeJavaScript('document.fonts.ready.then(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))))');
  await new Promise((r) => setTimeout(r, 500));
  let image = await win.webContents.capturePage();
  if (image.getSize().width !== WIDTH) image = image.resize({ width: WIDTH, height: HEIGHT, quality: 'best' });
  fs.writeFileSync(out, image.toPNG());
  console.log(`captured ${image.getSize().width}x${image.getSize().height} -> ${out}`);
  win.destroy();
}

app.on('window-all-closed', () => {});
app
  .whenReady()
  .then(startServer)
  .then(capture)
  .catch((e) => {
    console.error(String(e?.stack || e));
    process.exitCode = 1;
  })
  .finally(() => stop().finally(() => (process.exitCode ? app.exit(1) : app.quit())));
