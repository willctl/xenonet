// The installer (`xenolith install`), end to end in a sandbox. Its config-merge helpers have unit
// tests next to them in rust/crates/cli/src/install/.

import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import fsp from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { buildIcons } from '../scripts/build-icons.mjs';
import { rustBin } from './impl.mjs';

test('the installer: a sandboxed install merges into your files, re-runs as a no-op, and uninstalls cleanly', { timeout: 180_000 }, async () => {
  const repo = fileURLToPath(new URL('..', import.meta.url));
  if (!fs.existsSync(path.join(repo, 'dist', 'icons', 'xenolith-healthy.ico'))) buildIcons({ outDir: path.join(repo, 'dist', 'icons'), preview: false });
  const sb = await fsp.mkdtemp(path.join(os.tmpdir(), 'xenolith-rust-install-'));
  // The payload a release ships: both programs, the window's files and the icons, in one folder.
  const from = path.join(sb, 'payload');
  const exe = process.platform === 'win32' ? '.exe' : '';
  await fsp.mkdir(from, { recursive: true });
  for (const name of [`xenolith${exe}`, `xenolith-tray${exe}`, 'WebView2Loader.dll']) {
    const src = path.join(path.dirname(rustBin()), name);
    if (fs.existsSync(src)) await fsp.copyFile(src, path.join(from, name));
  }
  await fsp.cp(path.join(repo, 'dist', 'runtime', 'ui'), path.join(from, 'ui'), { recursive: true });
  await fsp.cp(path.join(repo, 'dist', 'icons'), path.join(from, 'icons'), { recursive: true });
  const bin = path.join(from, `xenolith${exe}`);
  const run = (cmd) => spawnSync(bin, [cmd, '--sandbox', sb, '--from', from], { encoding: 'utf8', timeout: 120_000 });
  try {
    // Your own files: instructions with your text, and a Claude desktop config (CRLF) with another server.
    const claudeMd = path.join(sb, '.claude', 'CLAUDE.md');
    await fsp.mkdir(path.dirname(claudeMd), { recursive: true });
    await fsp.writeFile(claudeMd, '# Mine\n\nBe terse.\n');
    const desktopDir =
      process.platform === 'win32' ? path.join(sb, 'AppData', 'Roaming', 'Claude') : process.platform === 'darwin' ? path.join(sb, 'Library', 'Application Support', 'Claude') : path.join(sb, '.config', 'Claude');
    const desktop = path.join(desktopDir, 'claude_desktop_config.json');
    await fsp.mkdir(desktopDir, { recursive: true });
    const mine = '{\r\n  "mcpServers": {\r\n    "other": {\r\n      "command": "x"\r\n    }\r\n  }\r\n}\r\n';
    await fsp.writeFile(desktop, mine);

    let r = run('install');
    assert.equal(r.status, 0, r.stdout + r.stderr);
    assert.match(r.stdout, /MCP over stdio OK: xenolith \d+\.\d+\.\d+, 5 tools, context_start OK/);
    assert.match(r.stdout, /hook OK under/);
    const text = fs.readFileSync(desktop, 'utf8');
    assert.ok(text.includes('\r\n'), 'your line endings are kept');
    const d = JSON.parse(text);
    assert.deepEqual(Object.keys(d.mcpServers), ['other', 'xenolith']);
    assert.deepEqual(d.mcpServers['xenolith'].args, ['serve']);
    assert.ok(fs.existsSync(`${desktop}.bak-xenolith`), 'backed up once');
    assert.match(fs.readFileSync(claudeMd, 'utf8'), /^# Mine\n\nBe terse\.\n\n<!-- XENOLITH:START/);
    const findRendered = (dir) => {
      for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
        const p = path.join(dir, e.name);
        if (e.isDirectory() && e.name === 'xenolith' && fs.existsSync(path.join(p, '.mcp.json'))) return p;
        if (e.isDirectory()) {
          const f = findRendered(p);
          if (f) return f;
        }
      }
      return null;
    };
    const rendered = findRendered(sb);
    const mcp = JSON.parse(fs.readFileSync(path.join(rendered, '.mcp.json'), 'utf8'));
    assert.match(mcp.mcpServers['xenolith'].command, /xenolith(\.exe)?$/);
    assert.match(JSON.parse(fs.readFileSync(path.join(rendered, 'hooks', 'hooks.json'), 'utf8')).hooks.SessionStart[0].hooks[0].command, / hook$/);
    assert.ok(fs.existsSync(path.join(sb, 'Xenolith', 'index.md')), 'the wiki is set up');

    r = run('install');
    assert.equal(r.status, 0, r.stdout + r.stderr);
    assert.match(r.stdout, /xenolith(\.exe)?: unchanged/);
    assert.match(r.stdout, /managed block already current/);

    // While Claude desktop runs (in the sandbox: this file), its config is left alone. Nothing for you to
    // do when it already has this install's entry; a step for you when it does not.
    const running = path.join(sb, 'claude-desktop-running');
    await fsp.writeFile(running, '');
    const current = fs.readFileSync(desktop, 'utf8');
    r = run('install');
    assert.equal(r.status, 0, r.stdout + r.stderr);
    assert.match(r.stdout, /ok\s+Claude desktop MCP: already registered/);
    assert.equal(fs.readFileSync(desktop, 'utf8'), current);
    const stale = JSON.parse(current);
    stale.mcpServers['xenolith'].command = 'C:/old/xenolith.exe';
    await fsp.writeFile(desktop, JSON.stringify(stale));
    r = run('install');
    assert.equal(r.status, 0, r.stdout + r.stderr);
    assert.match(r.stdout, /manual\s+Claude desktop MCP: quit Claude desktop/);
    assert.equal(JSON.parse(fs.readFileSync(desktop, 'utf8')).mcpServers['xenolith'].command, 'C:/old/xenolith.exe', 'not edited while it runs');
    await fsp.writeFile(desktop, current);
    await fsp.rm(running);

    r = run('uninstall');
    assert.equal(r.status, 0, r.stdout + r.stderr);
    assert.equal(fs.readFileSync(claudeMd, 'utf8'), '# Mine\n\nBe terse.\n', 'your text, without the block');
    assert.equal(fs.readFileSync(desktop, 'utf8'), mine, 'your config as it was');
    assert.ok(fs.existsSync(path.join(sb, 'Xenolith', 'index.md')), 'the wiki is never deleted');

    // Another app's config that is valid JSON but not an object stops the install and names the file.
    await fsp.writeFile(desktop, '[]');
    r = run('install');
    assert.notEqual(r.status, 0);
    assert.match(r.stdout + r.stderr, /claude_desktop_config\.json is not a JSON object/);
    assert.doesNotMatch(r.stderr, /panicked/);
    assert.equal(fs.readFileSync(desktop, 'utf8'), '[]', 'left as it was');
  } finally {
    await fsp.rm(sb, { recursive: true, force: true, maxRetries: 5 });
  }
});
