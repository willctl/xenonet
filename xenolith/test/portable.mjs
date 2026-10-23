// What differs between Windows, Linux and macOS for the program: case-sensitive file names and
// symlinked paths. Lock liveness and how codex is started and stopped have unit tests in
// rust/crates/core.

import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import fsp from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { after, describe, test } from 'node:test';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { rustBin, srvT } from './impl.mjs';

const REPO = fileURLToPath(new URL('..', import.meta.url));
const WIN = process.platform === 'win32';
const tmp = await fsp.mkdtemp(path.join(os.tmpdir(), 'xenolith-portable-'));
after(() => fsp.rm(tmp, { recursive: true, force: true, maxRetries: 5 }).catch(() => {}));

describe('files and paths', () => {
  test('context_read finds INDEX.md and Xenolith on a case-sensitive filesystem', async () => {
    const wikiDir = path.join(tmp, 'wiki-case');
    const env = { ...process.env, XENOLITH_DIR: wikiDir, XENOLITH_HOME: path.join(tmp, 'home-case'), XENOLITH_WRITE_MODE: 'direct' };
    const client = new Client({ name: 'portable-test', version: '1' });
    await client.connect(new StdioClientTransport({ ...srvT(), env, stderr: 'ignore' }));
    try {
      const w = await client.callTool({ name: 'context_upsert_page', arguments: { app: 'test', title: 'Xenolith', type: 'project', summary: 'test page', content: '# Xenolith\n\nHello.', mode: 'replace' } });
      assert.ok(!w.isError, w.content?.[0]?.text);
      for (const target of ['INDEX.md', 'Xenolith', 'protocol.md']) {
        const r = await client.callTool({ name: 'context_read', arguments: { target } });
        assert.ok(!r.isError, `${target}: ${r.content?.[0]?.text}`);
      }
    } finally {
      await client.close();
    }
  });

  test('the curator starts from a symlinked folder, with an npm-style codex (node beside it, as npm installs them) and no node on PATH', { skip: WIN && 'symlinks need privileges on Windows' }, async () => {
    // macOS: os.tmpdir() is under /var, a symlink to /private/var.
    const real = path.join(tmp, 'real-runtime');
    await fsp.mkdir(real, { recursive: true });
    await fsp.copyFile(rustBin(), path.join(real, 'xenolith'));
    const link = path.join(tmp, 'linked-runtime');
    await fsp.symlink(real, link, 'dir');
    const npmBin = path.join(tmp, 'npm-prefix-bin');
    await fsp.mkdir(npmBin);
    await fsp.symlink(process.execPath, path.join(npmBin, 'node'));
    const codex = path.join(npmBin, 'codex');
    const fake = pathToFileURL(path.join(REPO, 'test', 'fixtures', 'fake-codex.mjs')).href;
    await fsp.writeFile(codex, `#!/usr/bin/env node\nimport(${JSON.stringify(fake)});\n`, { mode: 0o755 });
    const home = path.join(tmp, 'home-link');
    await fsp.mkdir(home, { recursive: true });
    await fsp.writeFile(path.join(home, 'config.json'), JSON.stringify({ curator: { codexPath: codex, codexHome: path.join(home, 'codex-home') } }));
    const r = spawnSync(path.join(link, 'xenolith'), ['curator', '--login-status'], {
      encoding: 'utf8',
      env: { HOME: home, XENOLITH_HOME: home, XENOLITH_DIR: path.join(tmp, 'wiki-link'), PATH: '/nonexistent' },
      timeout: 30_000,
    });
    assert.equal(r.status, 0, r.stderr);
    assert.deepEqual(JSON.parse(r.stdout.trim().split('\n').pop()).signedIn, true, r.stdout + r.stderr);
  });
});
