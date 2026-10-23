// Entry sync between two machines (the Rust programs only), as two Xenolith instances on this
// one: "studio" curates project:atlas and serves it over HTTP; "laptop" follows it, pushes the
// notes its sessions leave there and pulls studio's pages. A page replaced against an older
// version gets 409 and waits on studio as a proposal; nothing is lost on either side.

import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js';
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import fsp from 'node:fs/promises';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { after, before, describe, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { IMPL, curatorCmd, rustBin, srv, srvT } from './impl.mjs';

const skip = IMPL !== 'rust' && 'the Rust programs only';
const repo = fileURLToPath(new URL('..', import.meta.url));
const runtime = path.join(repo, 'dist', 'runtime');
const fakeCodex = path.join(repo, 'test', 'fixtures', 'fake-codex.mjs');
const tmp = await fsp.mkdtemp(path.join(os.tmpdir(), 'xenolith-sync-'));
after(() => fsp.rm(tmp, { recursive: true, force: true, maxRetries: 5 }).catch(() => {}));

function machine(name) {
  const dir = path.join(tmp, name);
  const home = path.join(dir, 'home');
  fs.mkdirSync(home, { recursive: true });
  fs.writeFileSync(path.join(home, 'config.json'), JSON.stringify({ writeMode: 'curated', curator: { codexPath: fakeCodex, lint: 'off', debounceSeconds: 0.2, maxWaitSeconds: 1, maxAttempts: 2, timeoutSeconds: 30 } }));
  const env = { ...process.env, XENOLITH_DIR: path.join(dir, 'wiki'), XENOLITH_HOME: home };
  delete env.XENOLITH_TOKEN;
  delete env.XENOLITH_WRITE_MODE;
  const atlas = path.join(dir, 'wiki', 'spaces', 'project', 'atlas');
  return { name, dir, home, env, wikiDir: path.join(dir, 'wiki'), atlas };
}

function cli(m, args, input) {
  const r = spawnSync(rustBin(), args, { env: m.env, encoding: 'utf8', input });
  return { code: r.status, out: r.stdout, err: r.stderr, json: () => JSON.parse(r.stdout) };
}

function ok(m, args, input) {
  const r = cli(m, args, input);
  assert.equal(r.code, 0, `${args.join(' ')}: ${r.err}${r.out}`);
  return r;
}

function startHttp(env) {
  return new Promise((resolve, reject) => {
    const proc = spawn(...srv(runtime, '--http', '--port', '0', '--parent-stdin'), { env, stdio: ['pipe', 'pipe', 'pipe'] });
    let err = '';
    proc.stderr.on('data', (d) => {
      err += d;
      const m = err.match(/listening on http:\/\/127\.0\.0\.1:(\d+)\/mcp/);
      if (m) resolve({ proc, port: Number(m[1]), base: `http://127.0.0.1:${m[1]}` });
    });
    proc.on('exit', (code) => reject(new Error(`server exited ${code}:\n${err}`)));
  });
}

function request(port, method, p, { body, headers = {} } = {}) {
  return new Promise((resolve, reject) => {
    const data = body === undefined ? undefined : JSON.stringify(body);
    const req = http.request({ host: '127.0.0.1', port, method, path: p, headers: { ...(data ? { 'Content-Type': 'application/json' } : {}), ...headers } }, (res) => {
      let text = '';
      res.setEncoding('utf8');
      res.on('data', (d) => (text += d));
      res.on('end', () => resolve({ status: res.statusCode, json: text ? JSON.parse(text) : null }));
    });
    req.on('error', reject);
    if (data) req.write(data);
    req.end();
  });
}

async function session(m, fn) {
  const c = new Client({ name: 'sync-test', version: '1' });
  await c.connect(new StdioClientTransport({ ...srvT(runtime), env: m.env, stderr: 'pipe' }));
  try {
    return await fn(async (name, args) => {
      const r = await c.callTool({ name, arguments: args });
      const text = r.content.map((x) => x.text).join('\n');
      assert.equal(Boolean(r.isError), false, `${name}: ${text}`);
      return text;
    });
  } finally {
    await c.close();
  }
}

const md = (dir) => (fs.existsSync(dir) ? fs.readdirSync(dir).filter((f) => f.endsWith('.md')) : []);
const page = (title, body) => `---\ntitle: ${title}\ntype: project\n---\n\n# ${title}\n\n${body}\n`;
const walk = (dir) => (fs.existsSync(dir) ? fs.readdirSync(dir, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? walk(path.join(dir, e.name)) : [path.join(dir, e.name)])) : []);

describe('sync between two machines', { skip, timeout: 120_000 }, () => {
  const studio = machine('studio');
  const laptop = machine('laptop');
  let server;
  let token;
  before(async () => {
    ok(studio, ['serve', '--init']);
    ok(studio, ['spaces', 'add', 'project:atlas']);
    fs.writeFileSync(path.join(studio.atlas, 'pages', 'db.md'), page('Atlas database', 'Postgres 16 on the studio box.'));
    ok(studio, ['grant', 'machine:laptop', 'ctx:write:project:atlas']);
    token = ok(studio, ['token', 'create', 'machine:laptop', '--json']).json().token;
    server = await startHttp(studio.env);
    ok(laptop, ['serve', '--init']);
    ok(laptop, ['sync', 'name', 'laptop']);
    ok(laptop, ['sync', 'peer', 'add', 'studio', server.base], token);
    ok(laptop, ['sync', 'follow', 'project:atlas', 'studio']);
  });
  after(() => server?.proc.kill());

  test('setup refuses what would leak or misroute', () => {
    assert.match(cli(laptop, ['sync', 'peer', 'add', 'far', 'http://far.example'], token).err, /must be https:\/\//);
    assert.match(cli(laptop, ['sync', 'peer', 'add', 'x', server.base], 'not a token').err, /give the peer's token for this machine on stdin/);
    assert.match(cli(laptop, ['sync', 'follow', 'machine:local', 'studio']).err, /always curated here/);
    assert.match(cli(laptop, ['sync', 'follow', 'project:x', 'nobody']).err, /no peer named nobody/);
    const status = ok(laptop, ['sync', 'status', '--json']).json();
    assert.deepEqual(status, { machine: 'laptop', peers: [{ name: 'studio', url: server.base, token: true }], follow: { 'project:atlas': 'studio' } });
    const tokenFile = path.join(laptop.home, 'peers', 'studio.token');
    if (process.platform !== 'win32') assert.equal(fs.statSync(tokenFile).mode & 0o777, 0o600);
    assert.ok(!fs.readFileSync(path.join(laptop.home, 'sync.json'), 'utf8').includes(token), 'the token stays out of sync.json');
  });

  test("a note left on the laptop reaches the studio's inbox once, and the studio's pages come back", async () => {
    await session(laptop, (call) => call('context_log', { app: 'codex', space: 'project:atlas', title: 'Atlas moves to Postgres 17', body: 'Logical replication.' }));
    assert.equal(md(path.join(laptop.atlas, 'inbox')).length, 1);
    const r = ok(laptop, ['sync', '--json']).json();
    assert.deepEqual(
      r.spaces.map((s) => [s.space, s.peer, s.pushed, s.conflicts, s.refused, s.pulled]),
      [['project:atlas', 'studio', 1, 0, 0, 1]],
    );
    assert.equal(md(path.join(laptop.atlas, 'inbox')).length, 0, "it left the laptop's inbox");
    const sent = walk(path.join(laptop.atlas, '.curator', 'sent'));
    assert.equal(sent.length, 1);
    assert.match(fs.readFileSync(sent[0], 'utf8'), /^<!-- sent to studio at \S+: \{"id":"[^"]+","result":"accepted","note":"[^"]+"\} -->\n---\n/);
    const arrived = md(path.join(studio.atlas, 'inbox'));
    assert.equal(arrived.length, 1);
    const note = fs.readFileSync(path.join(studio.atlas, 'inbox', arrived[0]), 'utf8');
    assert.match(note, /^title: Atlas moves to Postgres 17$/m);
    assert.match(note, /^transport: sync$/m);
    assert.match(note, /^client: "sync machine:laptop"$/m);
    assert.equal(fs.readFileSync(path.join(laptop.atlas, 'pages', 'db.md'), 'utf8'), fs.readFileSync(path.join(studio.atlas, 'pages', 'db.md'), 'utf8'), 'the page came back');
    assert.deepEqual(ok(laptop, ['sync', '--json']).json().spaces.map((s) => [s.pushed, s.pulled]), [[0, 0]], 'nothing new');
  });

  test("the laptop's curator leaves the followed space alone", async () => {
    await session(laptop, (call) => call('context_log', { app: 'codex', space: 'project:atlas', title: 'Waiting for the studio', body: 'x', pages: ['db'] }));
    const run = spawnSync(...curatorCmd(runtime, '--once'), { env: laptop.env, encoding: 'utf8' });
    assert.equal(run.status, 0, run.stderr);
    assert.equal(md(path.join(laptop.atlas, 'inbox')).length, 1, 'still waiting to be pushed');
    assert.equal(ok(laptop, ['sync', '--json']).json().spaces[0].pushed, 1);
  });

  test('a page replaced against an older version gets 409 and waits on the studio as a proposal', async () => {
    // The laptop's session sees the page as it was pulled, and replaces it.
    await session(laptop, (call) => call('context_upsert_page', { app: 'codex', space: 'project:atlas', title: 'Atlas database', slug: 'db', mode: 'replace', content: 'Postgres 17 on the laptop.' }));
    // Meanwhile the studio's page changes.
    fs.writeFileSync(path.join(studio.atlas, 'pages', 'db.md'), page('Atlas database', 'Postgres 16, with a replica now.'));
    const r = ok(laptop, ['sync', '--json']).json().spaces[0];
    assert.deepEqual([r.pushed, r.conflicts, r.pulled], [0, 1, 1]);
    assert.match(fs.readFileSync(path.join(laptop.atlas, 'pages', 'db.md'), 'utf8'), /with a replica now/, "the laptop shows the studio's page");
    assert.match(fs.readFileSync(path.join(studio.atlas, 'pages', 'db.md'), 'utf8'), /with a replica now/, "the studio's page is untouched");
    const held = ok(studio, ['held', '--space', 'project:atlas', '--json']).json().changes;
    assert.equal(held.length, 1);
    assert.equal(held[0].kind, 'sync');
    assert.equal(held[0].slug, 'db');
    assert.match(held[0].reasons[0], /^machine:laptop \(tok_[0-9a-f]{12}\) wrote this against version [0-9a-f]{16}, but the page is at [0-9a-f]{16} now\.$/);
    assert.match(ok(studio, ['held', '--space', 'project:atlas']).out, /proposal from another machine: Atlas database \[\[db\]\]/);
    // The window's API sees it in its space. Approving needs the owner key: the window's header
    // alone is refused, and `xenolith held approve` reads the key and asks the service.
    const api = await request(server.port, 'GET', '/api/held?space=project:atlas');
    assert.equal(api.json.changes.length, 1);
    const refused = await request(server.port, 'POST', '/api/held', { headers: { 'X-Xenolith': 'ui' }, body: { space: 'project:atlas', batch: held[0].batch, index: 0, action: 'approve' } });
    assert.equal(refused.status, 403, JSON.stringify(refused.json));
    assert.match(cli(studio, ['held', 'approve', `${held[0].batch}:0`]).err, /no held changes for batch/, 'a proposal waits in its own space');
    const done = ok(studio, ['held', 'approve', `${held[0].batch}:0`, '--space', 'project:atlas', '--json']).json();
    assert.equal(done.status, 'approved');
    assert.match(fs.readFileSync(path.join(studio.atlas, 'pages', 'db.md'), 'utf8'), /Postgres 17 on the laptop\./);
    const decision = fs.readFileSync(path.join(studio.home, 'audit', 'chain.jsonl'), 'utf8').trim().split('\n').map((l) => JSON.parse(l)).filter((l) => l.kind === 'decision').pop();
    assert.deepEqual([decision.who, decision.space, decision.action, decision.outcome], ['cli', 'project:atlas', 'approve', 'approved']);
    assert.equal(ok(laptop, ['sync', '--json']).json().spaces[0].pulled, 1, 'and the laptop pulls the approved page');
    assert.match(fs.readFileSync(path.join(laptop.atlas, 'pages', 'db.md'), 'utf8'), /Postgres 17 on the laptop\./);
    assert.ok(walk(path.join(laptop.atlas, '.history')).length >= 1, 'the laptop keeps what each pull replaced');
  });

  test('a replace against the current page is a note for the curator, not a proposal', async () => {
    await session(laptop, (call) => call('context_upsert_page', { app: 'codex', space: 'project:atlas', title: 'Atlas database', slug: 'db', mode: 'replace', content: 'Postgres 17, tuned.' }));
    const r = ok(laptop, ['sync', '--json']).json().spaces[0];
    assert.deepEqual([r.pushed, r.conflicts], [1, 0]);
    assert.equal(ok(studio, ['held', '--space', 'project:atlas', '--json']).json().changes.length, 0);
  });

  test('the studio takes entries only from what it granted, and only for spaces it curates', async () => {
    const entry = { id: '2026-10-16_23-00-00-000-abc123', note: { kind: 'log', app: 'x', title: 'x', body: 'x' } };
    const auth = { Authorization: `Bearer ${token}` };
    assert.equal((await request(server.port, 'GET', '/sync/v1/spaces/machine/local/pages', { headers: auth })).status, 404, 'not granted reads as not found');
    assert.equal((await request(server.port, 'GET', '/sync/v1/spaces/project/nope/pages', { headers: auth })).status, 404);
    assert.equal((await request(server.port, 'GET', '/sync/v1/spaces/project/atlas/pages', { headers: { Authorization: `Bearer xenolith_${'0'.repeat(64)}` } })).status, 401);
    ok(studio, ['grant', 'machine:reader', 'ctx:read:project:atlas']);
    const reader = ok(studio, ['token', 'create', 'machine:reader', '--json']).json().token;
    const r = await request(server.port, 'POST', '/sync/v1/spaces/project/atlas/entries', { headers: { Authorization: `Bearer ${reader}` }, body: { entries: [entry] } });
    assert.equal(r.status, 200);
    assert.deepEqual([r.json.results[0].result, r.json.results[0].error], ['refused', "This token can't append here."]);
    assert.equal((await request(server.port, 'POST', '/sync/v1/spaces/project/atlas/entries', { headers: auth, body: { entries: [] } })).status, 400);
    // The laptop only follows project:atlas: it sends a pusher to the curator. Sync always takes a
    // token, even from the same machine.
    const laptopServer = await startHttp(laptop.env);
    try {
      assert.equal((await request(laptopServer.port, 'POST', '/sync/v1/spaces/project/atlas/entries', { body: { entries: [entry] } })).status, 401);
      ok(laptop, ['grant', 'machine:phone', 'ctx:append:project:atlas']);
      const phone = ok(laptop, ['token', 'create', 'machine:phone', '--json']).json().token;
      const misdirected = await request(laptopServer.port, 'POST', '/sync/v1/spaces/project/atlas/entries', { headers: { Authorization: `Bearer ${phone}` }, body: { entries: [entry] } });
      assert.equal(misdirected.status, 421);
      assert.deepEqual([misdirected.json.error, misdirected.json.curator], ['not_curator', 'studio']);
    } finally {
      laptopServer.proc.kill();
    }
  });

  test('both machines keep the sync in their audit chains', () => {
    const lines = (m) => fs.readFileSync(path.join(m.home, 'audit', 'chain.jsonl'), 'utf8').trim().split('\n').map((l) => JSON.parse(l));
    const studioSync = lines(studio).filter((l) => l.kind === 'sync');
    assert.ok(studioSync.some((l) => l.conflicts === 1 && /^machine:laptop \(tok_/.test(l.who)));
    assert.ok(lines(laptop).some((l) => l.kind === 'sync' && l.peer === 'studio' && l.as === 'machine:laptop' && l.pushed >= 1));
    assert.equal(cli(studio, ['audit', 'verify']).code, 0);
    assert.equal(cli(laptop, ['audit', 'verify']).code, 0);
  });
});
