// The tray app (xenolith-tray) and its icons (any OS). The tray under test uses its own instance
// name and a temp folder, so it never touches a real installed tray.

import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js';
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import fsp from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { after, before, describe, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { SIZES, buildIcons } from '../scripts/build-icons.mjs';
import { currentUserSid, deleteTask, queryTask, regValue, taskXmlBytes, trayTaskXml } from '../scripts/lib.mjs';
import { rustBin, srv } from './impl.mjs';

const WIN = process.platform === 'win32';
const repo = fileURLToPath(new URL('..', import.meta.url));
const tmp = await fsp.mkdtemp(path.join(os.tmpdir(), 'xenolith-tray-'));
after(() => fsp.rm(tmp, { recursive: true, force: true, maxRetries: 5 }).catch(() => {}));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function readIco(file) {
  const b = fs.readFileSync(file);
  assert.equal(b.readUInt16LE(2), 1, 'type icon');
  const n = b.readUInt16LE(4);
  return Array.from({ length: n }, (_, i) => {
    const e = 6 + i * 16;
    const size = b[e] || 256;
    const off = b.readUInt32LE(e + 12);
    const len = b.readUInt32LE(e + 8);
    const data = b.subarray(off, off + len);
    return { size, bpp: b.readUInt16LE(e + 6), png: data.subarray(0, 4).toString('latin1') === '\x89PNG', data };
  });
}

/** RGBA of pixel (x, y) in a 32-bit DIB icon frame (bottom-up BGRA after a 40-byte header). */
function dibPixel(frame, x, y) {
  const s = frame.size;
  const o = 40 + ((s - 1 - y) * s + x) * 4;
  return [frame.data[o + 2], frame.data[o + 1], frame.data[o], frame.data[o + 3]];
}

describe('icons', () => {
  const out = path.join(tmp, 'icons');
  test('three states, each a multi-size .ico (16-256 px); the 16 px page-bubble is pixel-sharp', () => {
    const files = buildIcons({ outDir: out });
    assert.deepEqual(files.map((f) => path.basename(f)), ['xenolith-healthy.ico', 'xenolith-degraded.ico', 'xenolith-down.ico']);
    for (const f of files) {
      const frames = readIco(f);
      assert.deepEqual(frames.map((x) => x.size), SIZES);
      for (const fr of frames) {
        assert.equal(fr.bpp, 32);
        assert.equal(fr.png, fr.size === 256, `${fr.size} px frame format`);
      }
    }
    const [healthy, degraded, down] = files.map((f) => readIco(f)[0]);
    const INDIGO = [0x4f, 0x46, 0xe5, 255];
    assert.deepEqual(dibPixel(healthy, 4, 4), [255, 255, 255, 255], 'text line is solid white');
    assert.deepEqual(dibPixel(healthy, 5, 10), INDIGO, 'the page is solid indigo');
    assert.deepEqual(dibPixel(healthy, 4, 13), INDIGO, 'the speech-bubble tail');
    assert.deepEqual(dibPixel(healthy, 11, 4), [0xa5, 0xb4, 0xfc, 255], 'the folded corner');
    assert.equal(dibPixel(healthy, 13, 2)[3], 0, 'transparent past the fold: no tile');
    assert.equal(dibPixel(healthy, 0, 0)[3], 0, 'transparent corner');
    assert.equal(dibPixel(healthy, 12, 12)[3], 0, 'no badge when healthy (the corner below the page is empty)');
    assert.deepEqual(dibPixel(healthy, 9, 9), [255, 255, 255, 255], 'no knockout ring when healthy');
    assert.deepEqual(dibPixel(degraded, 12, 12), [0xf5, 0x9e, 0x0b, 255], 'amber badge');
    assert.ok(dibPixel(degraded, 9, 9)[3] < 128, 'a ring is knocked out around the badge');
    assert.deepEqual(dibPixel(down, 12, 12), [0xdc, 0x26, 0x26, 255], 'red badge');
    assert.deepEqual(dibPixel(down, 5, 10), [0x6b, 0x72, 0x80, 255], 'grey page when down');
    assert.ok(fs.statSync(path.join(out, 'preview.png')).size > 1000);
  });
});

describe('tray app', { timeout: 180_000 }, () => {
  const exeName = WIN ? 'xenolith-tray.exe' : 'xenolith-tray';
  const agentName = WIN ? 'xenolith.exe' : 'xenolith';
  const dir = path.join(tmp, 'rust-tray');
  const exe = path.join(dir, exeName);
  const agent = path.join(dir, 'runtime', agentName);
  const wikiDir = path.join(dir, 'wiki');
  const home = path.join(dir, 'home');
  const env = { ...process.env, XENOLITH_DIR: wikiDir, XENOLITH_HOME: home };
  delete env.XENOLITH_WRITE_MODE;
  const ini = path.join(dir, 'xenolith-tray.ini');
  const testKey = `Software\\XenolithTest-rust-${process.pid}`;
  const runKey = `${testKey}\\Run`;
  const taskName = `XenolithTest-rust-${process.pid}`;
  const taskXml = path.join(dir, 'test.task.xml');
  const trayLog = path.join(home, 'logs', 'tray.log');
  const logText = () => (fs.existsSync(trayLog) ? fs.readFileSync(trayLog, 'utf8') : '');
  let server;
  let port;
  const writeIni = (extra = []) =>
    fs.writeFileSync(
      ini,
      [
        `agent=${agent}`,
        `wikiDir=${wikiDir}`,
        `logDir=${path.join(home, 'logs')}`,
        `icons=${path.join(tmp, 'icons')}`,
        `webviewDir=${path.join(home, 'webview')}`,
        `port=${port}`,
        `instance=XenolithTray-rust-test-${process.pid}`,
        `runKey=${runKey}`,
        'runValue=XenolithTray',
        `task=${taskName}`,
        `taskXml=${taskXml}`,
        `codex=${path.join(repo, 'test', 'fixtures', 'fake-codex.mjs')}`,
        `codexHome=${path.join(home, 'codex-home')}`,
        ...extra,
        '',
      ].join('\n'),
    );
  const selftest = () => {
    const r = spawnSync(exe, ['--selftest', '--config', ini], { encoding: 'utf8', timeout: 20_000, env });
    assert.equal(r.status, 0, r.stderr);
    return r.stdout;
  };
  const run = (action) => spawnSync(exe, ['--do', action, '--config', ini], { encoding: 'utf8', timeout: 60_000, env });
  const startTray = (extra) => {
    writeIni(extra);
    const p = spawn(exe, ['--config', ini, '--from', 'test'], { env, stdio: 'ignore' });
    return { p, exited: new Promise((r) => (p.exitCode !== null ? r(p.exitCode) : p.on('exit', r))) };
  };
  const quit = () => spawnSync(exe, ['--quit', '--config', ini], { timeout: 40_000 }).status;

  before(async () => {
    fs.mkdirSync(path.join(dir, 'runtime'), { recursive: true });
    if (!fs.existsSync(path.join(tmp, 'icons'))) buildIcons({ outDir: path.join(tmp, 'icons'), preview: false });
    fs.copyFileSync(path.join(path.dirname(rustBin()), exeName), exe);
    // A GNU cross-build loads WebView2 from a DLL beside it (the MSVC build links it in).
    const loader = path.join(path.dirname(rustBin()), 'WebView2Loader.dll');
    if (WIN && fs.existsSync(loader)) fs.copyFileSync(loader, path.join(dir, 'WebView2Loader.dll'));
    fs.copyFileSync(rustBin(), agent);
    fs.mkdirSync(home, { recursive: true });
    fs.writeFileSync(
      path.join(home, 'config.json'),
      JSON.stringify({ curator: { codexPath: path.join(repo, 'test', 'fixtures', 'fake-codex.mjs'), lint: 'off', debounceSeconds: 0.2, maxWaitSeconds: 1, pollSeconds: 1 } }),
    );
    server = spawn(...srv('--http', '--port', '0', '--parent-stdin'), { env, stdio: ['pipe', 'pipe', 'pipe'] });
    port = await new Promise((resolve, reject) => {
      let err = '';
      server.stderr.on('data', (d) => {
        err += d;
        const m = err.match(/127\.0\.0\.1:(\d+)\/mcp/);
        if (m) resolve(Number(m[1]));
      });
      server.on('exit', () => reject(new Error(err)));
    });
  });
  after(() => server?.kill());

  test('--selftest: state, tooltip, icons, the window and the whole menu from /status', () => {
    writeIni(['curator=0']);
    const out = selftest();
    assert.match(out, /^state=healthy$/m);
    assert.match(out, /^tooltip=Xenolith \d+\.\d+\.\d+: OK · 0 queued$/m);
    assert.match(out, /^icons=3$/m);
    assert.match(out, new RegExp(`^ui=http://127\\.0\\.0\\.1:${port}/ui/$`, 'm'));
    // The service's owner key, which the tray uses once /status shows the pid that wrote it.
    const key = JSON.parse(fs.readFileSync(path.join(home, 'runtime', 'owner.key'), 'utf8'));
    assert.equal(key.port, port);
    assert.match(out, new RegExp(`^owner=ok \\(pid ${key.pid}\\)$`, 'm'));
    assert.match(out, /^window=webview \d+x\d+ /m, 'an embedded webview, not a browser window');
    assert.ok(out.includes(`data=${path.join(home, 'webview')}`), 'its own small data folder');
    assert.match(out.split('\n').find((l) => l.startsWith('item=')), /^item=Open Xenolith$/, 'the first menu item opens the window');
    for (const item of [
      /^item=Xenolith \d+\.\d+\.\d+ · OK$/m,
      /^item=Up \d+ s · 0 note\(s\) queued$/m,
      /^item=Open folder$/m,
      /^item=Open index\.md$/m,
      /^item=Open logs$/m,
      /^item=Recent activity > /m,
      /^item=Pause curator$/m,
      /^item=Curator > Sign in to ChatGPT for the curator\.\.\.$/m,
      /^item=Curator > Retry failed notes \(0\)$/m,
      new RegExp(`^item=Copy MCP URL \\(http://127\\.0\\.0\\.1:${port}/mcp\\)$`, 'm'),
      /^item=Restart service$/m,
      /^item=Quit$/m,
    ]) {
      assert.match(out, item);
    }
    fs.mkdirSync(path.join(wikiDir, '.curator'), { recursive: true });
    fs.writeFileSync(path.join(wikiDir, '.curator', 'paused'), 'test\n');
    assert.match(selftest(), /^item=Resume curator$/m);
    fs.rmSync(path.join(wikiDir, '.curator', 'paused'));
  });

  test('--do runs the menu actions: pause/resume, copy URL, failed notes, restart service', () => {
    writeIni(['curator=0', `service=${WIN ? 'XenolithTestNoSuchService' : 'xenolith-test-no-such.service'}`]);
    const flag = path.join(wikiDir, '.curator', 'paused');
    let r = run('pause');
    assert.equal(r.status, 0, r.stdout);
    assert.match(r.stdout, /^ok: Curator paused/);
    assert.ok(fs.existsSync(flag));
    assert.match(selftest(), /^item=Resume curator$/m);
    assert.equal(run('resume').status, 0);
    assert.ok(!fs.existsSync(flag));
    if (WIN) {
      // A locked workstation (or a clipboard tool holding it) denies the clipboard to everyone: check only when it works.
      const ps = (cmd) => spawnSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', cmd], { encoding: 'utf8' }).stdout.trim();
      const probe = ps("Set-Clipboard -Value 'xenolith-probe'; Get-Clipboard");
      r = run('copy-url');
      if (probe === 'xenolith-probe') {
        assert.equal(r.status, 0, r.stdout);
        assert.equal(ps('Get-Clipboard'), `http://127.0.0.1:${port}/mcp`);
      } else {
        assert.match(r.stdout, /^(ok: Copied|error: The clipboard is in use)/);
      }
    }
    assert.match(run('retry-dead').stdout, /^ok: Failed notes queued for another try\./);
    assert.match(run('file-raw').stdout, /^ok: Failed notes filed into the log/);
    r = run('restart-service');
    assert.equal(r.status, 1);
    assert.match(r.stdout, /^error: /, 'a service it cannot control is reported, not thrown');
    r = run('no-such-action');
    assert.equal(r.status, 1);
    assert.match(r.stdout, /^error: Unknown action: no-such-action/);
  });

  test('single instance; a second start asks for the window; --do open-ui reaches the running tray; --quit stops it', { skip: !WIN && 'needs a desktop session' }, async () => {
    const { p, exited } = startTray(['curator=0', 'icon=0', 'window=0']);
    await sleep(1500);
    assert.equal(p.exitCode, null, 'first instance keeps running');
    const second = spawnSync(exe, ['--config', ini], { timeout: 20_000 });
    assert.equal(second.status, 3, 'second instance exits: already running');
    const r = run('open-ui');
    assert.equal(r.status, 0, r.stdout);
    assert.match(r.stdout, /^ok: open-ui$/m);
    const opened = () => (logText().match(/open-ui requested \(window=0\)/g) || []).length;
    for (let i = 0; i < 50 && opened() < 2; i++) await sleep(100);
    assert.equal(opened(), 2, 'the second start and --do open-ui both reached it');
    assert.equal(quit(), 0);
    assert.equal(await Promise.race([exited, sleep(10_000).then(() => 'timeout')]), 0);
    assert.equal(quit(), 0, '--quit with nothing running is fine');
    assert.match(logText(), /\[tray\] quit$/m);
  });

  test('start at sign-in: reports the logon task and Run entry, Repair registers both, a vanished one is logged, never re-added', { skip: !WIN && 'Windows entries' }, async () => {
    fs.writeFileSync(taskXml, taskXmlBytes(trayTaskXml({ exe, sid: currentUserSid(), enabled: false })));
    let tray;
    try {
      writeIni(['curator=0', 'icon=0', 'window=0', 'autostartCheckSeconds=1']);
      let out = selftest();
      assert.match(out, /^autostart=task:missing run:missing$/m);
      assert.match(out, /^item=! Will not start at sign-in: Start at sign-in > Repair$/m);
      assert.match(out, /^item=Start at sign-in > Logon task \(Task Scheduler\): missing$/m);
      assert.match(out, /^item=Start at sign-in > Run key entry \(HKCU\): missing$/m);
      assert.match(out, /^item=Start at sign-in > Repair: register both again$/m);
      const r = run('repair-autostart');
      assert.equal(r.status, 0, r.stdout);
      assert.match(r.stdout, /^ok: Xenolith starts at sign-in again \(task:disabled run:on\)\./);
      assert.match(regValue(`HKCU\\${runKey}`, 'XenolithTray'), /^"[^"]*xenolith-tray\.exe" --from run$/);
      assert.equal(queryTask(taskName).exists, true);
      out = selftest();
      assert.match(out, /^autostart=task:disabled run:on$/m);
      assert.match(out, /^item=! Start at sign-in: logon task disabled$/m);
      assert.match(logText(), /\[tray\] start at sign-in repaired: task:disabled run:on/);
      tray = startTray(['curator=0', 'icon=0', 'window=0', 'autostartCheckSeconds=1']);
      for (let i = 0; i < 50 && !/\[tray\] start at sign-in: task:disabled run:on/.test(logText()); i++) await sleep(200);
      assert.match(logText(), /\[tray\] started pid \d+ \(from test\): .*xenolith-tray\.exe/);
      assert.equal(spawnSync('reg.exe', ['delete', `HKCU\\${runKey}`, '/v', 'XenolithTray', '/f']).status, 0);
      for (let i = 0; i < 50 && !/changed: task:disabled run:on -> task:disabled run:missing/.test(logText()); i++) await sleep(200);
      assert.match(logText(), /\[tray\] start at sign-in changed: task:disabled run:on -> task:disabled run:missing \(between \d{4}-\d\d-\d\dT[\d:]+ and now\)/);
      await sleep(2500); // two more checks
      assert.equal(regValue(`HKCU\\${runKey}`, 'XenolithTray'), null, 'the tray does not put an entry back by itself');
    } finally {
      if (tray) {
        quit();
        await Promise.race([tray.exited, sleep(10_000)]);
      }
      deleteTask(taskName);
      spawnSync('reg.exe', ['delete', `HKCU\\${testKey}`, '/f']);
    }
    assert.equal(queryTask(taskName).exists, false);
  });

  test('hosts the curator as the user: files a note, restarts it on upgrade, stops it on quit', { skip: !WIN && 'needs a desktop session' }, async () => {
    const tray = startTray(['curator=1', 'icon=0', 'window=0']);
    const client = new Client({ name: 'tray-test', version: '1' });
    await client.connect(new StreamableHTTPClientTransport(new URL(`http://127.0.0.1:${port}/mcp`)));
    const curatorLog = path.join(home, 'logs', 'curator.log');
    const clog = () => (fs.existsSync(curatorLog) ? fs.readFileSync(curatorLog, 'utf8') : '');
    try {
      const r = await client.callTool({ name: 'context_log', arguments: { app: 'tray-test', title: 'Filed by the hosted curator', body: 'x', pages: ['tray-rust'] } });
      assert.ok(!r.isError, r.content[0].text);
      const page = path.join(wikiDir, 'pages', 'tray-rust.md');
      for (let i = 0; i < 150 && !fs.existsSync(page); i++) await sleep(200);
      assert.ok(fs.existsSync(page), 'the hosted curator filed the note');
      assert.match(clog(), /\[tray\] started curator pid \d+/);

      // An upgrade: the running program is renamed aside and a new one copied in (what the installer does).
      fs.renameSync(agent, `${agent}.old`);
      fs.copyFileSync(rustBin(), agent);
      for (let i = 0; i < 100 && !/xenolith changed on disk/.test(clog()); i++) await sleep(200);
      assert.match(clog(), /xenolith changed on disk; restarting the curator/);
      for (let i = 0; i < 100 && (clog().match(/started curator pid/g) || []).length < 2; i++) await sleep(200);

      let menu = '';
      for (let i = 0; i < 50 && !/Curator: (idle|waiting|working)/.test(menu); i++) {
        await sleep(200);
        menu = selftest();
      }
      assert.match(menu, /^item=Curator > Curator: (idle|waiting|working) \(gpt-6\.1-sol, medium\)$/m, 'the tray sees its curator running');
    } finally {
      await client.close();
      assert.equal(quit(), 0);
      assert.equal(await Promise.race([tray.exited, sleep(30_000).then(() => 'timeout')]), 0);
    }
    assert.match(clog(), /\[tray\] curator host stopped/);
    const status = JSON.parse(fs.readFileSync(path.join(wikiDir, '.curator', 'status.json'), 'utf8'));
    assert.equal(status.state, 'stopped', 'the curator stopped cleanly');
  });
});
