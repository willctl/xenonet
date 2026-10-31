// Pairing (the Rust programs only): "home" curates and serves, "sat" joins it with a code and
// follows every space home lists. One code, one join, read and append only, and no secret in
// either machine's audit chain.

import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import fsp from 'node:fs/promises';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { after, before, describe, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { curatorCmd, rustBin, srv } from './impl.mjs';

const repo = fileURLToPath(new URL('..', import.meta.url));
const fakeCodex = path.join(repo, 'test', 'fixtures', 'fake-codex.mjs');
const tmp = await fsp.mkdtemp(path.join(os.tmpdir(), 'xenolith-pair-'));
after(() => fsp.rm(tmp, { recursive: true, force: true, maxRetries: 5 }).catch(() => {}));

function machine(name) {
  const dir = path.join(tmp, name);
  const home = path.join(dir, 'home');
  fs.mkdirSync(home, { recursive: true });
  fs.writeFileSync(path.join(home, 'config.json'), JSON.stringify({ writeMode: 'curated', curator: { codexPath: fakeCodex, lint: 'off', debounceSeconds: 0.2, maxWaitSeconds: 1, maxAttempts: 2, timeoutSeconds: 30 } }));
  const env = { ...process.env, XENOLITH_DIR: path.join(dir, 'wiki'), XENOLITH_HOME: home };
  delete env.XENOLITH_TOKEN;
  delete env.XENOLITH_WRITE_MODE;
  return { name, dir, home, env, chain: path.join(home, 'audit', 'chain.jsonl'), space: (s) => path.join(dir, 'wiki', 'spaces', ...s.split(':')) };
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

const page = (title, body) => `---\ntitle: ${title}\ntype: project\n---\n\n# ${title}\n\n${body}\n`;
const chainText = (m) => fs.readFileSync(m.chain, 'utf8');

describe('pairing a satellite with a home', { timeout: 180_000 }, () => {
  const home = machine('home');
  const sat = machine('sat');
  let server;
  let code;
  let token;

  before(async () => {
    ok(home, ['serve', '--init']);
    ok(home, ['spaces', 'add', 'project:atlas']);
    fs.writeFileSync(path.join(home.space('project:atlas'), 'pages', 'db.md'), page('Atlas database', 'Postgres 16 at home.'));
    server = await startHttp(home.env);
    ok(sat, ['serve', '--init']);
  });
  after(() => server?.proc.kill());

  test('an offer needs the owner key and prints one code, valid ten minutes', async () => {
    const refused = await request(server.port, 'POST', '/api/pair/offer', { body: {}, headers: { 'X-Xenolith': 'cli' } });
    assert.equal(refused.status, 403, 'a local process without the owner key cannot make a code');
    const offer = ok(home, ['pair', 'offer', '--json']).json();
    assert.match(offer.code, /^[0-9a-f]{16}\.[0-9a-f]{32}$/);
    assert.equal(offer.expiresInSeconds, 600);
    code = offer.code;
    if (process.platform !== 'win32') assert.equal(fs.statSync(path.join(home.home, 'pair.json')).mode & 0o777, 0o600, 'the offer is yours alone');
  });

  test('a join works: the token is stored as peers/<name>.token and home is named', () => {
    assert.match(cli(sat, ['pair', 'join'], code).err, /--url/, 'the address is its own argument');
    const joined = ok(sat, ['pair', 'join', '--url', server.base, '--name', 'sat', '--json'], `${code}\n`).json();
    assert.equal(joined.home, 'home');
    assert.equal(joined.followedSpaces, 1);
    token = fs.readFileSync(path.join(sat.home, 'peers', 'home.token'), 'utf8').trim();
    assert.match(token, /^xenolith_[0-9a-f]{64}$/);
    if (process.platform !== 'win32') assert.equal(fs.statSync(path.join(sat.home, 'peers', 'home.token')).mode & 0o777, 0o600);
    const status = ok(sat, ['sync', 'status', '--json']).json();
    assert.equal(status.home, 'home');
    assert.deepEqual(status.follow, { 'project:atlas': 'home' });
    assert.match(ok(sat, ['sync', 'status']).out, /Satellite of home/);
    const cfg = JSON.parse(fs.readFileSync(path.join(sat.home, 'sync.json'), 'utf8'));
    assert.deepEqual(cfg.follow['project:atlas'], { peer: 'home', auto: true });
    assert.match(joined.token, /^tok_[0-9a-f]{12}$/);
  });

  test('the token reads and appends everywhere but machine:local, and writes nowhere', async () => {
    const policy = JSON.parse(fs.readFileSync(path.join(home.home, 'access.json'), 'utf8'));
    const rec = policy.tokens.find((t) => t.subject === 'machine:sat');
    assert.ok(rec, 'home keeps a token for machine:sat');
    assert.deepEqual([...rec.scopes].sort(), ['ctx:append', 'ctx:read']);
    const auth = { Authorization: `Bearer ${token}` };
    const pages = await request(server.port, 'GET', '/sync/v1/spaces/project/atlas/pages', { headers: auth });
    assert.equal(pages.status, 200);
    const list = await request(server.port, 'GET', '/sync/v1/spaces', { headers: auth });
    assert.deepEqual(list.json.spaces, ['project:atlas'], 'machine:local is never listed');
    assert.equal((await request(server.port, 'GET', '/sync/v1/spaces/machine/local/pages', { headers: auth })).status, 404, 'machine:local stays out of reach');
    const base = pages.json.pages.find((p) => p.slug === 'db').hash;
    const entry = (n, mode) => ({ id: `2026-01-01_00-00-00-000-${n}`, note: { kind: 'page', app: 'sync', title: 'DB', body: 'x', page: { slug: 'db', mode, base } } });
    const r = await request(server.port, 'POST', '/sync/v1/spaces/project/atlas/entries', { headers: auth, body: { entries: [entry('aaaaaa', 'replace')] } });
    assert.equal(r.json.results[0].result, 'refused', 'replacing a page needs write');
    assert.equal(r.json.results[0].status, 403);
  });

  test('a replayed code is refused, and the first token still works', async () => {
    const again = cli(sat, ['pair', 'join', '--url', server.base, '--name', 'other', '--peer', 'second'], code);
    assert.notEqual(again.code, 0);
    assert.match(again.err, /already used/);
    assert.ok(!fs.existsSync(path.join(sat.home, 'peers', 'second.token')));
    const auth = { Authorization: `Bearer ${token}` };
    assert.equal((await request(server.port, 'GET', '/sync/v1/spaces', { headers: auth })).status, 200);
    assert.match(chainText(home), /pair code already used/);
  });

  test('a space made at home later is followed at the next sync, and explicit follows stay explicit', () => {
    ok(home, ['spaces', 'add', 'project:later']);
    fs.writeFileSync(path.join(home.space('project:later'), 'pages', 'plan.md'), page('Later plan', 'Made after pairing.'));
    const run = ok(sat, ['sync', '--json']).json();
    assert.deepEqual(run.spaces.map((s) => s.space).sort(), ['project:atlas', 'project:later']);
    assert.ok(run.spaces.every((s) => !s.error), JSON.stringify(run));
    assert.match(fs.readFileSync(path.join(sat.space('project:later'), 'pages', 'plan.md'), 'utf8'), /Made after pairing/);
    assert.match(fs.readFileSync(path.join(sat.space('project:atlas'), 'pages', 'db.md'), 'utf8'), /Postgres 16 at home/);
    ok(sat, ['sync', 'follow', 'project:atlas', 'home']);
    const cfg = JSON.parse(fs.readFileSync(path.join(sat.home, 'sync.json'), 'utf8'));
    assert.deepEqual(cfg.follow['project:atlas'], { peer: 'home' }, 'an explicit follow is not home-driven');
    assert.deepEqual(cfg.follow['project:later'], { peer: 'home', curator: cfg.follow['project:later'].curator, auto: true });
    ok(sat, ['sync']);
    assert.equal(JSON.parse(fs.readFileSync(path.join(sat.home, 'sync.json'), 'utf8')).follow['project:atlas'].auto, undefined);
  });

  test("a satellite's curator and Ask are off", async () => {
    const proc = spawn(...curatorCmd('--parent-stdin'), { env: sat.env, stdio: ['pipe', 'pipe', 'pipe'] });
    let err = '';
    proc.stderr.on('data', (d) => (err += d));
    for (let i = 0; i < 100 && !/satellite/.test(err); i++) await new Promise((r) => setTimeout(r, 100));
    proc.stdin.end();
    await new Promise((r) => proc.on('exit', r));
    assert.match(err, /satellite: files nothing and answers nothing/);
    assert.ok(!/curating project:/.test(err), err);
  });

  test('neither audit chain holds the code or the token, and both name the token id', () => {
    const second = ok(home, ['pair', 'offer', '--json']).json().code;
    const [, secret2] = second.split('.');
    const [, secret] = code.split('.');
    for (const m of [home, sat]) {
      const text = chainText(m);
      for (const s of [secret, secret2, token, token.slice('xenolith_'.length)]) assert.ok(!text.includes(s), `${m.name}'s chain holds a secret`);
      assert.equal(JSON.parse(ok(m, ['audit', 'verify', '--json']).out).broken ?? null, null);
    }
    const joined = chainText(home).split('\n').filter(Boolean).map((l) => JSON.parse(l)).find((e) => e.kind === 'pair' && e.event === 'joined');
    assert.match(joined.token, /^tok_[0-9a-f]{12}$/);
    assert.equal(joined.subject, 'machine:sat');
    const paired = chainText(sat).split('\n').filter(Boolean).map((l) => JSON.parse(l)).find((e) => e.kind === 'pair' && e.event === 'paired');
    assert.equal(paired.token, joined.token, 'both machines name the same token');
  });

  test('a wrong secret is refused and, repeated, rate-limited like bad tokens', async () => {
    const offer = ok(home, ['pair', 'offer', '--json']).json().code;
    const [homeId] = offer.split('.');
    const wrong = `${homeId}.${'0'.repeat(32)}`;
    const first = cli(sat, ['pair', 'join', '--url', server.base, '--name', 'intruder', '--peer', 'intruder'], wrong);
    assert.notEqual(first.code, 0);
    assert.match(first.err, /not valid/);
    assert.ok(!fs.existsSync(path.join(sat.home, 'peers', 'intruder.token')));
    let last;
    for (let i = 0; i < 25; i++) {
      last = await request(server.port, 'POST', '/sync/v1/pair', { body: { v: 1, home: homeId, name: 'intruder', key: '11'.repeat(32), proof: '22'.repeat(32) } });
      if (last.status === 429) break;
      assert.equal(last.status, 403);
    }
    assert.equal(last.status, 429);
    const blocked = cli(sat, ['pair', 'join', '--url', server.base, '--name', 'late', '--peer', 'late'], offer);
    assert.match(blocked.err, /429|Too many/, 'the real code is refused too while the source is blocked');
    assert.match(chainText(home), /too many bad pairing codes/);
  });
});
