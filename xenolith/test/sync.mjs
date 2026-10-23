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
import { curatorCmd, rustBin, srv, srvT } from './impl.mjs';

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
    const proc = spawn(...srv('--http', '--port', '0', '--parent-stdin'), { env, stdio: ['pipe', 'pipe', 'pipe'] });
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
  await c.connect(new StdioClientTransport({ ...srvT(), env: m.env, stderr: 'pipe' }));
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

describe('sync between two machines', { timeout: 120_000 }, () => {
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
    assert.deepEqual(status, {
      machine: 'laptop',
      peers: [{ name: 'studio', url: server.base, token: true }],
      follow: { 'project:atlas': 'studio' },
      timeoutSeconds: 90,
      spaces: [{ space: 'project:atlas', peer: 'studio', curator: null, dead: 0, waiting: 0, last: null }],
      dead: 0,
    });
    assert.equal(ok(laptop, ['sync', 'timeout', '120', '--json']).json().timeoutSeconds, 120);
    assert.equal(JSON.parse(fs.readFileSync(path.join(laptop.home, 'sync.json'), 'utf8')).timeoutSeconds, 120, 'a peer behind a forwarder may need longer');
    assert.match(cli(laptop, ['sync', 'timeout', '1']).err, /from 5 to 600/);
    const tokenFile = path.join(laptop.home, 'peers', 'studio.token');
    if (process.platform !== 'win32') {
      assert.equal(fs.statSync(tokenFile).mode & 0o777, 0o600);
      assert.equal(fs.statSync(path.join(laptop.home, 'sync.json')).mode & 0o777, 0o600, 'sync.json is yours alone too');
    }
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
    const run = spawnSync(...curatorCmd('--once'), { env: laptop.env, encoding: 'utf8' });
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
      assert.deepEqual([misdirected.json.error, misdirected.json.curator, misdirected.json.curatorUrl], ['not_curator', 'studio', null], 'a loopback URL means nothing on another machine');
      const copy = await request(laptopServer.port, 'GET', '/sync/v1/spaces/project/atlas/pages', { headers: { Authorization: `Bearer ${phone}` } });
      assert.equal(copy.status, 421, 'a copy is not listed as if it were the curator');
      // With an address another machine can use, the answer names it.
      const cfg = JSON.parse(fs.readFileSync(path.join(laptop.home, 'sync.json'), 'utf8'));
      fs.writeFileSync(path.join(laptop.home, 'sync.json'), JSON.stringify({ ...cfg, peers: { studio: 'https://studio.example/xenolith' } }));
      const named = await request(laptopServer.port, 'POST', '/sync/v1/spaces/project/atlas/entries', { headers: { Authorization: `Bearer ${phone}` }, body: { entries: [entry] } });
      fs.writeFileSync(path.join(laptop.home, 'sync.json'), JSON.stringify(cfg));
      assert.equal(named.json.curatorUrl, 'https://studio.example/xenolith');
      assert.match(named.json.error_description, /its curator is studio, at https:\/\/studio\.example\/xenolith\.$/);
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

// A forwarder in front of a curator, as a tunnel or a gateway would be: it serves the curator
// under a path prefix (which it strips) and can be told to fail chosen requests.
function forwarder(target, prefix) {
  const rules = [];
  const seen = [];
  const server = http.createServer((req, res) => {
    seen.push(`${req.method} ${req.url}`);
    for (const rule of rules) {
      const answer = rule(req);
      if (answer) {
        req.resume();
        res.writeHead(answer.status, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify(answer.body ?? {}));
        return;
      }
    }
    if (!req.url.startsWith(`${prefix}/`)) {
      res.writeHead(404).end();
      return;
    }
    const up = http.request({ host: '127.0.0.1', port: target, method: req.method, path: req.url.slice(prefix.length), headers: { ...req.headers, host: `127.0.0.1:${target}` } }, (r) => {
      res.writeHead(r.statusCode, r.headers);
      r.pipe(res);
    });
    up.on('error', () => res.writeHead(502).end());
    req.pipe(up);
  });
  return new Promise((resolve) =>
    server.listen(0, '127.0.0.1', () => {
      const { port } = server.address();
      resolve({ base: `http://127.0.0.1:${port}${prefix}`, rules, seen, close: () => server.close() });
    }),
  );
}

describe('a curator and two followers', { timeout: 600_000 }, () => {
  const hub = machine('hub');
  const alpha = machine('alpha'); // may write, through a forwarder under /hub
  const beta = machine('beta'); // may only append, straight to the hub
  let server;
  let fwd;
  const today = new Date(Date.now() - new Date().getTimezoneOffset() * 60_000).toISOString().slice(0, 10);
  const logRel = path.join('log', today.slice(0, 4), `${today}.md`);
  // One `xenolith sync`, run without blocking this process: the forwarder answers in it.
  const report = (m) =>
    new Promise((resolve, reject) => {
      const p = spawn(rustBin(), ['sync', '--json'], { env: m.env, stdio: ['ignore', 'pipe', 'pipe'] });
      let out = '';
      let err = '';
      p.stdout.on('data', (d) => (out += d));
      p.stderr.on('data', (d) => (err += d));
      p.on('error', reject);
      p.on('close', (code) => {
        try {
          resolve({ ...JSON.parse(out).spaces[0], code });
        } catch {
          reject(new Error(`sync --json (${code}): ${err}${out}`));
        }
      });
    });
  const hubPage = (slug) => path.join(hub.atlas, 'pages', `${slug}.md`);
  const copyPage = (m, slug) => path.join(m.atlas, 'pages', `${slug}.md`);
  const read = (f) => fs.readFileSync(f, 'utf8');
  const follow = (m, base, tok) => {
    ok(m, ['serve', '--init']);
    ok(m, ['sync', 'name', m.name]);
    ok(m, ['sync', 'peer', 'add', 'hub', base], tok);
    ok(m, ['sync', 'follow', 'project:atlas', 'hub']);
  };

  before(async () => {
    ok(hub, ['serve', '--init']);
    ok(hub, ['spaces', 'add', 'project:atlas']);
    fs.writeFileSync(hubPage('db'), page('Atlas database', 'Postgres 16.'));
    fs.writeFileSync(hubPage('guide'), page('Atlas guide', 'Start with the README.'));
    fs.mkdirSync(path.dirname(path.join(hub.atlas, logRel)), { recursive: true });
    fs.writeFileSync(path.join(hub.atlas, logRel), `# ${today}\n\n- 09:00 · curator · Filed the database notes [[db]]\n`);
    ok(hub, ['grant', 'machine:alpha', 'ctx:write:project:atlas']);
    ok(hub, ['grant', 'machine:beta', 'ctx:append:project:atlas']);
    const alphaToken = ok(hub, ['token', 'create', 'machine:alpha', '--json']).json().token;
    const betaToken = ok(hub, ['token', 'create', 'machine:beta', '--json']).json().token;
    server = await startHttp(hub.env);
    fwd = await forwarder(server.port, '/hub');
    follow(alpha, fwd.base, alphaToken);
    follow(beta, server.base, betaToken);
  });
  after(() => {
    server?.proc.kill();
    fwd?.close();
  });

  test('the first pull pins the curator and copies its pages and recent log, through a path prefix', async () => {
    const r = await report(alpha);
    assert.equal(r.code, 0, JSON.stringify(r));
    assert.deepEqual([r.pulled, r.logs, r.error], [2, 1, undefined]);
    assert.ok(r.logBytes > 0 && r.logBytes < 1000, `the log came along: ${r.logBytes} bytes`);
    const id = JSON.parse(read(path.join(hub.atlas, '.curator', 'sync', 'curator.json'))).id;
    assert.match(id, /^[0-9a-f]{32}$/);
    assert.deepEqual(JSON.parse(read(path.join(alpha.home, 'sync.json'))).follow, { 'project:atlas': { peer: 'hub', curator: id } }, 'pinned at the first pull');
    assert.ok(fwd.seen.length > 0 && fwd.seen.every((s) => / \/hub\/sync\/v1\/spaces\/project\/atlas\/(pages|log)/.test(s)), fwd.seen.join('\n'));
    assert.equal(read(copyPage(alpha, 'guide')), read(hubPage('guide')));
    assert.equal(read(path.join(alpha.atlas, logRel)), read(path.join(hub.atlas, logRel)));
    const start = await session(alpha, (call) => call('context_start', { app: 'codex', space: 'project:atlas' }));
    assert.match(start, /Filed the database notes/, "the copy's context_start shows the curator's recent activity");
    assert.equal((await report(beta)).pulled, 2);
    const again = await report(alpha);
    assert.deepEqual([again.pulled, again.logs], [0, 0], 'nothing new, nothing fetched');
  });

  test('a note the curator refuses for good becomes a dead letter and is not sent again', async () => {
    // beta may only append: a page it replaces is refused (403).
    await session(beta, (call) => call('context_upsert_page', { app: 'codex', space: 'project:atlas', title: 'Atlas guide', slug: 'guide', mode: 'replace', content: 'Rewritten on beta.' }));
    // And a note no curator can take (a page note with no content) is invalid (400).
    const bad = '2026-10-22_08-00-00-000-badbad';
    fs.writeFileSync(path.join(beta.atlas, 'inbox', `${bad}.md`), `---\nid: ${bad}\nkind: page\napp: codex\ntitle: Empty\npage_slug: empty\npage_mode: append\n---\n\n`);
    const r = await report(beta);
    assert.deepEqual([r.pushed, r.refused, r.dead, r.waiting], [0, 2, 2, 0], JSON.stringify(r));
    const deadDir = path.join(beta.atlas, '.curator', 'sync', 'dead');
    const texts = md(deadDir).map((f) => read(path.join(deadDir, f)));
    assert.equal(texts.length, 2);
    assert.ok(texts.some((t) => /^<!-- refused by hub at \S+: This token can't write here\. -->\n---\n/.test(t)), texts.join('\n'));
    assert.ok(texts.some((t) => /^<!-- refused by hub at \S+: `content` is required\. -->/.test(t)), texts.join('\n'));
    const again = await report(beta);
    assert.deepEqual([again.refused, again.dead], [0, 2], 'never sent again');
    const status = ok(beta, ['sync', 'status', '--json']).json();
    assert.deepEqual([status.dead, status.spaces[0].dead], [2, 2]);
    assert.match(ok(beta, ['sync', 'status']).out, /project:atlas, curated by hub \(curator [0-9a-f]{8}\): 0 note\(s\) waiting, 2 dead letter\(s\)/);
    const betaServer = await startHttp(beta.env);
    try {
      const s = (await request(betaServer.port, 'GET', '/status')).json;
      assert.equal(s.sync.dead, 2);
      assert.ok(s.reasons.includes('2 note(s) refused by a curator for good (xenolith sync status)'), JSON.stringify(s.reasons));
    } finally {
      betaServer.proc.kill();
    }
  });

  test('a failed push still pulls, and a refused token stops both', async () => {
    await session(alpha, (call) => call('context_log', { app: 'codex', space: 'project:atlas', title: 'Index rebuilt', body: 'x' }));
    fwd.rules.push((req) => req.method === 'POST' && { status: 503, body: { error: 'gateway busy' } });
    fs.writeFileSync(hubPage('db'), page('Atlas database', 'Postgres 16, tuned.'));
    let r = await report(alpha);
    assert.equal(r.code, 1);
    assert.match(r.error, /^push: hub answered 503: gateway busy/);
    assert.deepEqual([r.pushed, r.pulled, r.waiting], [0, 1, 1], 'the pull ran');
    assert.match(read(copyPage(alpha, 'db')), /tuned/);
    fwd.rules[0] = (req) => req.method === 'POST' && { status: 401, body: { error: 'invalid_token', error_description: 'The token is unknown.' } };
    fs.writeFileSync(hubPage('db'), page('Atlas database', 'Postgres 16, tuned twice.'));
    r = await report(alpha);
    assert.equal(r.errorKind, 'auth');
    assert.match(r.error, /^hub refused this machine's token \(401: The token is unknown\.\): nothing was pushed or pulled/);
    assert.deepEqual([r.pushed, r.pulled], [0, 0], 'no pull with a refused token');
    fwd.rules.length = 0;
    r = await report(alpha);
    assert.deepEqual([r.code, r.pushed, r.pulled, r.waiting], [0, 1, 1, 0]);
  });

  test('a page that vanishes mid-pull is skipped, and the rest is pulled', async () => {
    fs.writeFileSync(hubPage('p1'), page('P1', 'one'));
    fs.writeFileSync(hubPage('p2'), page('P2', 'two'));
    fwd.rules.push((req) => req.method === 'GET' && req.url.endsWith('/pages/p1') && { status: 404, body: { error: 'not_found' } });
    let r = await report(alpha);
    assert.deepEqual([r.code, r.skipped, r.pulled], [0, 1, 1], JSON.stringify(r));
    assert.ok(!fs.existsSync(copyPage(alpha, 'p1')) && fs.existsSync(copyPage(alpha, 'p2')));
    fwd.rules.length = 0;
    r = await report(alpha);
    assert.deepEqual([r.skipped, r.pulled], [0, 1]);
    assert.equal((await report(beta)).pulled, 3);
  });

  test("a follower's own edit goes to the curator, and against an older version waits there as a proposal", async () => {
    // Edited in alpha's copy, on the version alpha pulled: a note for the curator.
    fs.writeFileSync(copyPage(alpha, 'guide'), page('Atlas guide', 'Start with the README, then docs/.'));
    let r = await report(alpha);
    assert.deepEqual([r.pushed, r.conflicts], [1, 0], JSON.stringify(r));
    const notes = md(path.join(hub.atlas, 'inbox')).map((f) => read(path.join(hub.atlas, 'inbox', f)));
    const edit = notes.find((n) => /then docs\//.test(n));
    assert.ok(edit, 'the edit reached the hub as a note');
    assert.match(edit, /^page_mode: replace$/m);
    assert.match(edit, /^by: "machine:alpha"$/m);
    assert.equal(read(copyPage(alpha, 'guide')), read(hubPage('guide')), 'the copy shows the curator again until it files the note');
    assert.ok(walk(path.join(alpha.atlas, '.history', 'pages', 'guide')).some((f) => /then docs\//.test(read(f))), 'and keeps the edit in .history/');
    // Edited in alpha's copy while the hub's page moved on: a proposal on the hub.
    fs.writeFileSync(copyPage(alpha, 'db'), page('Atlas database', 'Postgres 17 on alpha.'));
    fs.writeFileSync(hubPage('db'), page('Atlas database', 'Postgres 16 with a replica.'));
    r = await report(alpha);
    assert.deepEqual([r.pushed, r.conflicts, r.pulled], [0, 1, 1], JSON.stringify(r));
    const held = ok(hub, ['held', '--space', 'project:atlas', '--json']).json().changes.filter((c) => c.slug === 'db');
    assert.equal(held.length, 1);
    assert.equal(held[0].kind, 'sync');
    assert.match(held[0].reasons[0], /^machine:alpha \(tok_[0-9a-f]{12}\) wrote this against version [0-9a-f]{16}, but the page is at [0-9a-f]{16} now\.$/);
    assert.match(read(copyPage(alpha, 'db')), /with a replica/);
    ok(hub, ['held', 'reject', `${held[0].batch}:0`, '--space', 'project:atlas']);
  });

  test('a removal travels both ways, and one from an older version waits as a proposal', async () => {
    // Removed on the hub: both copies remove it, keeping it in .history/.
    fs.rmSync(hubPage('guide'));
    for (const m of [alpha, beta]) {
      const r = await report(m);
      assert.deepEqual([r.removed, r.pushed], [1, 0], `${m.name}: ${JSON.stringify(r)}`);
      assert.ok(!fs.existsSync(copyPage(m, 'guide')));
      assert.ok(walk(path.join(m.atlas, '.history', 'pages', 'guide')).length >= 1);
    }
    // Removed in alpha's copy: the hub removes it too, and beta follows.
    fs.rmSync(copyPage(alpha, 'p2'));
    let r = await report(alpha);
    assert.deepEqual([r.pushed, r.removed], [1, 0], JSON.stringify(r));
    assert.ok(!fs.existsSync(hubPage('p2')), 'removed on the hub');
    assert.ok(!fs.existsSync(copyPage(alpha, 'p2')), 'and it stays removed in the copy');
    assert.match(read(path.join(hub.atlas, logRel)), /Removed P2 \[\[p2\]\], as machine:alpha \(tok_[0-9a-f]{12}\) did/);
    assert.equal((await report(beta)).removed, 1);
    assert.ok(!fs.existsSync(copyPage(beta, 'p2')));
    // Removed in alpha's copy while the hub changed it: a proposal, and the page comes back.
    fs.rmSync(copyPage(alpha, 'p1'));
    fs.writeFileSync(hubPage('p1'), page('P1', 'one, revised'));
    r = await report(alpha);
    assert.deepEqual([r.conflicts, r.pulled], [1, 1], JSON.stringify(r));
    assert.match(read(copyPage(alpha, 'p1')), /revised/, 'the page comes back');
    const held = ok(hub, ['held', '--space', 'project:atlas', '--json']).json().changes.filter((c) => c.slug === 'p1');
    assert.deepEqual([held.length, held[0].action, held[0].kind], [1, 'removed', 'sync']);
    assert.match(held[0].reasons[0], /^machine:alpha \(tok_[0-9a-f]{12}\) removed this page from version [0-9a-f]{16}, but the page is at [0-9a-f]{16} now\.$/);
    ok(hub, ['held', 'approve', `${held[0].batch}:0`, '--space', 'project:atlas']);
    assert.ok(!fs.existsSync(hubPage('p1')), 'approved: removed');
    assert.equal((await report(alpha)).removed, 1);
    // beta may not remove: refused for good, and the page comes back to beta's copy.
    fs.rmSync(copyPage(beta, 'db'));
    r = await report(beta);
    assert.deepEqual([r.refused, r.pulled], [1, 1], JSON.stringify(r));
    assert.ok(fs.existsSync(copyPage(beta, 'db')));
  });

  test('a backlog over 1,000 drains in one pass; a smaller one goes a batch a pass', async (t) => {
    const write = (m, n, tag) => {
      const dir = path.join(m.atlas, 'inbox');
      for (let i = 0; i < n; i++) {
        const id = `2026-10-22_10-00-00-000-${tag}${i.toString(36).padStart(5, '0')}`;
        fs.writeFileSync(path.join(dir, `${id}.md`), `---\nid: ${id}\nkind: log\napp: codex\nsource: agent\ntitle: Backlog note ${i}\n---\n\nBacklog body ${i}.\n`);
      }
    };
    const before = md(path.join(hub.atlas, 'inbox')).length;
    write(alpha, 2500, 'a');
    const t0 = Date.now();
    const r = await report(alpha);
    const ms = Date.now() - t0;
    assert.deepEqual([r.code, r.pushed, r.refused, r.waiting], [0, 2500, 0, 0], JSON.stringify(r));
    assert.equal(md(path.join(hub.atlas, 'inbox')).length, before + 2500);
    t.diagnostic(`2,500 notes pushed in ${ms} ms`);
    write(beta, 150, 'b');
    assert.deepEqual([(await report(beta)).pushed, (await report(beta)).pushed, (await report(beta)).pushed], [100, 50, 0]);
  });

  test('a follower that finds another curator for the space refuses to merge until told', async () => {
    const rival = machine('rival');
    ok(rival, ['serve', '--init']);
    ok(rival, ['spaces', 'add', 'project:atlas']);
    fs.writeFileSync(path.join(rival.atlas, 'pages', 'db.md'), page('Atlas database', 'A different story.'));
    ok(rival, ['grant', 'machine:beta', 'ctx:append:project:atlas']);
    const tok = ok(rival, ['token', 'create', 'machine:beta', '--json']).json().token;
    const rivalServer = await startHttp(rival.env);
    try {
      await session(beta, (call) => call('context_log', { app: 'codex', space: 'project:atlas', title: 'For the hub only', body: 'x' }));
      ok(beta, ['sync', 'peer', 'add', 'hub', rivalServer.base], tok); // the address now reaches another curator
      const before = read(copyPage(beta, 'db'));
      const r = await report(beta);
      assert.deepEqual([r.code, r.errorKind, r.pushed, r.pulled, r.waiting], [1, 'curator', 0, 0, 1]);
      assert.match(r.error, /two curators for one space, or the curator was set up again\. Nothing was pushed or pulled\. If hub is the right curator now, follow the space again: xenolith sync follow project:atlas hub$/);
      assert.equal(read(copyPage(beta, 'db')), before, 'nothing merged');
      assert.equal(md(path.join(rival.atlas, 'inbox')).length, 0, 'nothing sent');
      ok(beta, ['sync', 'follow', 'project:atlas', 'hub']);
      const again = await report(beta);
      assert.deepEqual([again.code, again.pushed, again.pulled], [0, 1, 1]);
      assert.match(read(copyPage(beta, 'db')), /A different story/);
    } finally {
      rivalServer.proc.kill();
    }
  });
});
