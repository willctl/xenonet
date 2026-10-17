// Pages as MCP resources, with update notifications, and the start rules as an MCP prompt (the
// Rust programs only). Resources follow the same access rules as the tools: a caller lists and
// reads the pages of the spaces it may read, and a page it may not read is "not found".

import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js';
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js';
import { ResourceListChangedNotificationSchema, ResourceUpdatedNotificationSchema } from '@modelcontextprotocol/sdk/types.js';
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import fsp from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { after, before, describe, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { IMPL, rustBin, srv, srvT } from './impl.mjs';

const skip = IMPL !== 'rust' && 'the Rust programs only';
const repo = fileURLToPath(new URL('..', import.meta.url));
const runtime = path.join(repo, 'dist', 'runtime');
const tmp = await fsp.mkdtemp(path.join(os.tmpdir(), 'xenolith-resources-'));
after(() => fsp.rm(tmp, { recursive: true, force: true, maxRetries: 5 }).catch(() => {}));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function cli(env, ...args) {
  const r = spawnSync(rustBin(), args, { env, encoding: 'utf8' });
  assert.equal(r.status, 0, r.stderr);
  return r.stdout;
}

const page = (title, summary, body) => `---\ntitle: ${title}\ntype: project\nsummary: ${summary}\n---\n\n# ${title}\n\n${body}\n`;

async function stdioClient(env) {
  const c = new Client({ name: 'resources-test', version: '1' });
  await c.connect(new StdioClientTransport({ ...srvT(runtime), env, stderr: 'pipe' }));
  return c;
}

async function waitFor(fn, ms, label) {
  const deadline = Date.now() + ms;
  for (;;) {
    const v = fn();
    if (v) return v;
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${label}`);
    await sleep(100);
  }
}

describe('pages as resources, and the start prompt', { skip, timeout: 60_000 }, () => {
  let E;
  let token; // read on project:atlas only
  before(() => {
    const dir = path.join(tmp, 'wiki-env');
    const home = path.join(dir, 'home');
    fs.mkdirSync(home, { recursive: true });
    fs.writeFileSync(path.join(home, 'config.json'), JSON.stringify({ writeMode: 'curated' }));
    E = { wikiDir: path.join(dir, 'wiki'), home, env: { ...process.env, XENOLITH_DIR: path.join(dir, 'wiki'), XENOLITH_HOME: home } };
    delete E.env.XENOLITH_TOKEN;
    cli(E.env, 'serve', '--init');
    cli(E.env, 'spaces', 'add', 'project:atlas');
    fs.writeFileSync(path.join(E.wikiDir, 'pages', 'home-lab.md'), page('Home lab', 'The rack in the hall', 'A 2.5 GbE switch. ![x](https://example.com/x.png)'));
    fs.writeFileSync(path.join(E.wikiDir, 'spaces', 'project', 'atlas', 'pages', 'db.md'), page('Atlas database', 'Postgres 17', 'Chosen for logical replication.'));
    cli(E.env, 'grant', 'client:phone', 'ctx:read:project:atlas');
    token = JSON.parse(cli(E.env, 'token', 'create', 'client:phone', '--json')).token;
  });

  test('every readable page is a resource, read as Markdown', async () => {
    const c = await stdioClient(E.env);
    try {
      const caps = c.getServerCapabilities();
      assert.deepEqual(caps.resources, { subscribe: true, listChanged: true });
      assert.deepEqual(caps.prompts, { listChanged: false });
      const { resources } = await c.listResources();
      assert.deepEqual(
        resources.map((r) => [r.uri, r.name, r.title, r.description, r.mimeType]),
        [
          ['xenolith://machine/local/pages/home-lab', 'machine:local/home-lab', 'Home lab', 'The rack in the hall', 'text/markdown'],
          ['xenolith://project/atlas/pages/db', 'project:atlas/db', 'Atlas database', 'Postgres 17', 'text/markdown'],
        ],
      );
      const { resourceTemplates } = await c.listResourceTemplates();
      assert.equal(resourceTemplates[0].uriTemplate, 'xenolith://{kind}/{name}/pages/{slug}');
      const r = await c.readResource({ uri: 'xenolith://machine/local/pages/home-lab' });
      assert.equal(r.contents[0].mimeType, 'text/markdown');
      assert.match(r.contents[0].text, /^---\ntitle: Home lab\n/);
      assert.doesNotMatch(r.contents[0].text, /!\[x\]\(https:/, 'remote images become plain links');
      for (const uri of ['xenolith://project/atlas/pages/nope', 'xenolith://project/other/pages/db', 'xenolith://machine/local/pages/../index', 'file:///etc/hosts']) {
        await assert.rejects(c.readResource({ uri }), (e) => e.code === -32002 && /Resource not found/.test(e.message), uri);
      }
    } finally {
      await c.close();
    }
  });

  test("a token's holder sees only the pages of its spaces", async () => {
    const c = await stdioClient({ ...E.env, XENOLITH_TOKEN: token });
    try {
      assert.deepEqual((await c.listResources()).resources.map((r) => r.uri), ['xenolith://project/atlas/pages/db']);
      assert.match((await c.readResource({ uri: 'xenolith://project/atlas/pages/db' })).contents[0].text, /logical replication/);
      await assert.rejects(c.readResource({ uri: 'xenolith://machine/local/pages/home-lab' }), (e) => e.code === -32002, 'the same answer as a page that is not there');
      await assert.rejects(c.getPrompt({ name: 'start', arguments: { space: 'machine:local' } }), /Refused: no space "machine:local" that you can read/);
    } finally {
      await c.close();
    }
  });

  test('a subscribed page that changes is announced, and so are pages that come or go', async () => {
    const c = await stdioClient(E.env);
    const updated = [];
    let listChanged = 0;
    c.setNotificationHandler(ResourceUpdatedNotificationSchema, (n) => updated.push(n.params.uri));
    c.setNotificationHandler(ResourceListChangedNotificationSchema, () => listChanged++);
    try {
      await c.subscribeResource({ uri: 'xenolith://project/atlas/pages/db' });
      await sleep(1500); // the watcher's first look sees the pages as they are
      fs.writeFileSync(path.join(E.wikiDir, 'spaces', 'project', 'atlas', 'pages', 'db.md'), page('Atlas database', 'Postgres 17', 'Chosen for logical replication. Upgraded on Friday.'));
      await waitFor(() => updated.includes('xenolith://project/atlas/pages/db'), 5000, 'resources/updated');
      fs.writeFileSync(path.join(E.wikiDir, 'pages', 'garden.md'), page('Garden', 'Beds and seeds', 'Tomatoes in bed 2.'));
      await waitFor(() => listChanged > 0, 5000, 'resources/list_changed');
      await c.unsubscribeResource({ uri: 'xenolith://project/atlas/pages/db' });
      const seen = updated.length;
      fs.writeFileSync(path.join(E.wikiDir, 'spaces', 'project', 'atlas', 'pages', 'db.md'), page('Atlas database', 'Postgres 17', 'Again.'));
      await sleep(2500);
      assert.equal(updated.length, seen, 'nothing after unsubscribing');
    } finally {
      await c.close();
      fs.rmSync(path.join(E.wikiDir, 'pages', 'garden.md'), { force: true });
    }
  });

  test('the start prompt is what context_start returns, rules first', async () => {
    const c = await stdioClient(E.env);
    try {
      const { prompts } = await c.listPrompts();
      assert.deepEqual(prompts.map((p) => [p.name, p.arguments.map((a) => a.name)]), [['start', ['app', 'topic', 'space']]]);
      const p = await c.getPrompt({ name: 'start', arguments: { app: 'claude-code', space: 'project:atlas' } });
      assert.equal(p.messages[0].role, 'user');
      const text = p.messages[0].content.text;
      assert.match(text, /# Xenolith protocol/);
      assert.match(text, /## 1\. Start every conversation with `context_start`/);
      assert.match(text, /^Space: project:atlas\. Other spaces you can use: machine:local \(admin\)\./m);
      assert.match(text, /- db \[project\] Atlas database - Postgres 17/);
      await assert.rejects(c.getPrompt({ name: 'other' }), /Unknown prompt/);
    } finally {
      await c.close();
    }
  });

  test('over stateless HTTP: resources and prompts, but no subscriptions', async () => {
    const proc = spawn(...srv(runtime, '--http', '--port', '0', '--parent-stdin'), { env: E.env, stdio: ['pipe', 'pipe', 'pipe'] });
    const url = await new Promise((resolve, reject) => {
      let err = '';
      proc.stderr.on('data', (d) => {
        err += d;
        const m = err.match(/listening on (http:\/\/127\.0\.0\.1:\d+\/mcp)/);
        if (m) resolve(m[1]);
      });
      proc.on('exit', (code) => reject(new Error(`exited ${code}: ${err}`)));
    });
    const c = new Client({ name: 'http', version: '1' });
    await c.connect(new StreamableHTTPClientTransport(new URL(url), { requestInit: { headers: { Authorization: `Bearer ${token}` } } }));
    try {
      assert.deepEqual(c.getServerCapabilities().resources, { subscribe: false, listChanged: false });
      assert.deepEqual((await c.listResources()).resources.map((r) => r.uri), ['xenolith://project/atlas/pages/db']);
      await assert.rejects(c.subscribeResource({ uri: 'xenolith://project/atlas/pages/db' }), /Method not found/);
    } finally {
      await c.close();
      proc.kill();
    }
  });

  test('the read-only server Ask uses offers tools only', async () => {
    const c = new Client({ name: 'codex-mcp-client', version: 'test' });
    await c.connect(new StdioClientTransport({ ...srvT(runtime, '--read-only'), env: E.env, stderr: 'pipe' }));
    try {
      const caps = c.getServerCapabilities();
      assert.equal(caps.resources, undefined);
      assert.equal(caps.prompts, undefined);
    } finally {
      await c.close();
    }
  });
});
