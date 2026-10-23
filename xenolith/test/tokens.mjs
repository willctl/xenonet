// Bearer tokens with ctx:* scopes (the Rust programs only): the HTTP service as an OAuth 2.1
// resource server the way MCP's authorization spec has it (resource metadata, 401 and 403
// challenges), XENOLITH_TOKEN for stdio, and the token commands. A token reaches the lower of
// what it asks (its scopes) and what its subject is granted, and nothing by default.

import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js';
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js';
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import fsp from 'node:fs/promises';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { after, before, describe, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { rustBin, srv, srvT } from './impl.mjs';

const repo = fileURLToPath(new URL('..', import.meta.url));
const runtime = path.join(repo, 'dist', 'runtime');
const tmp = await fsp.mkdtemp(path.join(os.tmpdir(), 'xenolith-tokens-'));
after(() => fsp.rm(tmp, { recursive: true, force: true, maxRetries: 5 }).catch(() => {}));

let n = 0;
function makeEnv({ access } = {}) {
  const dir = path.join(tmp, `case-${++n}`);
  const wikiDir = path.join(dir, 'wiki');
  const home = path.join(dir, 'home');
  fs.mkdirSync(home, { recursive: true });
  fs.writeFileSync(path.join(home, 'config.json'), JSON.stringify({ writeMode: 'curated' }));
  if (access) fs.writeFileSync(path.join(home, 'access.json'), JSON.stringify(access));
  const env = { ...process.env, XENOLITH_DIR: wikiDir, XENOLITH_HOME: home };
  delete env.XENOLITH_WRITE_MODE;
  delete env.XENOLITH_TOKEN;
  return { dir, wikiDir, home, env };
}

function cli(env, ...args) {
  const r = spawnSync(rustBin(), args, { env, encoding: 'utf8' });
  return { code: r.status, out: r.stdout, err: r.stderr };
}

function startHttp(env) {
  return new Promise((resolve, reject) => {
    const proc = spawn(...srv('--http', '--port', '0', '--parent-stdin'), { env, stdio: ['pipe', 'pipe', 'pipe'] });
    let err = '';
    const timer = setTimeout(() => reject(new Error(`server did not start:\n${err}`)), 10_000);
    proc.stderr.setEncoding('utf8');
    proc.stderr.on('data', (d) => {
      err += d;
      const m = err.match(/listening on (http:\/\/127\.0\.0\.1:(\d+)\/mcp)/);
      if (m) {
        clearTimeout(timer);
        resolve({ proc, url: m[1], port: Number(m[2]) });
      }
    });
    proc.on('exit', (code) => reject(new Error(`server exited ${code}:\n${err}`)));
  });
}

function request(port, { method = 'POST', path: p = '/mcp', headers = {}, body } = {}) {
  return new Promise((resolve, reject) => {
    const data = body === undefined ? undefined : typeof body === 'string' ? body : JSON.stringify(body);
    const req = http.request(
      { host: '127.0.0.1', port, method, path: p, headers: { ...(data ? { 'Content-Type': 'application/json', Accept: 'application/json, text/event-stream' } : {}), ...headers } },
      (res) => {
        let text = '';
        res.setEncoding('utf8');
        res.on('data', (d) => (text += d));
        res.on('end', () => {
          let json;
          try {
            json = JSON.parse(text);
          } catch {}
          resolve({ status: res.statusCode, headers: res.headers, text, json });
        });
      },
    );
    req.on('error', reject);
    if (data) req.write(data);
    req.end();
  });
}

const toolCall = (name, args) => ({ jsonrpc: '2.0', id: 1, method: 'tools/call', params: { name, arguments: args } });
const bearer = (t) => ({ Authorization: `Bearer ${t}` });
const textOf = (r) => r.json?.result?.content?.map((c) => c.text).join('\n') ?? '';

async function httpClient(url, token) {
  const c = new Client({ name: 'token-test', version: '1.0.0' });
  await c.connect(new StreamableHTTPClientTransport(new URL(url), { requestInit: { headers: token ? bearer(token) : {} } }));
  return c;
}

async function call(client, name, args) {
  const r = await client.callTool({ name, arguments: args });
  return { text: r.content.map((c) => c.text).join('\n'), isError: Boolean(r.isError) };
}

describe('tokens over HTTP', { timeout: 60_000 }, () => {
  let E;
  let server;
  let reader; // ctx:read on project:atlas, granted read there
  let writer; // asks ctx:write everywhere, granted append on its own space only
  before(async () => {
    E = makeEnv();
    for (const [space, title] of [['project:atlas', 'Atlas runs on Postgres 17'], [null, 'This machine has a home lab']]) {
      const c = new Client({ name: 'setup', version: '1' });
      await c.connect(new StdioClientTransport({ ...srvT(), env: E.env, stderr: 'pipe' }));
      await c.callTool({ name: 'context_log', arguments: { app: 'setup', title, body: 'x', ...(space ? { space } : {}) } });
      await c.close();
    }
    assert.equal(cli(E.env, 'grant', 'client:reader', 'ctx:read:project:atlas').code, 0);
    reader = JSON.parse(cli(E.env, 'token', 'create', 'client:reader', '--json').out).token;
    assert.equal(cli(E.env, 'grant', 'actor:hermes', 'ctx:append:by:hermes').code, 0);
    writer = JSON.parse(cli(E.env, 'token', 'create', 'actor:hermes', '--scope', 'ctx:write', '--json').out).token;
    server = await startHttp(E.env);
  });
  after(() => server?.proc.kill());

  test('the resource metadata names the endpoint and its scopes', async () => {
    for (const p of ['/.well-known/oauth-protected-resource', '/.well-known/oauth-protected-resource/mcp']) {
      const r = await request(server.port, { method: 'GET', path: p, headers: { Host: `127.0.0.1:${server.port}` } });
      assert.equal(r.status, 200);
      assert.equal(r.json.resource, `http://127.0.0.1:${server.port}/mcp`);
      assert.deepEqual(r.json.scopes_supported, ['ctx:read', 'ctx:append', 'ctx:write', 'ctx:admin']);
      assert.deepEqual(r.json.bearer_methods_supported, ['header']);
    }
  });

  test('a token reads what it was granted, and no other space exists for it', async () => {
    const c = await httpClient(server.url, reader);
    try {
      assert.doesNotMatch(c.getInstructions(), new RegExp(E.wikiDir.replace(/[\\/]/g, '.')), "no folder of this machine's");
      const start = await call(c, 'context_start', { app: 'phone' });
      assert.equal(start.isError, false, start.text);
      assert.match(start.text, /^Space: project:atlas\. No other space is open to you\.$/m, 'its only space is the default');
      assert.doesNotMatch(start.text, /Folder:/);
      assert.match((await call(c, 'context_search', { space: 'project:atlas', query: 'postgres' })).text, /Atlas runs on Postgres 17/);
      for (const space of ['machine:local', 'project:nope', 'actor:hermes']) {
        const r = await call(c, 'context_search', { space, query: 'home lab' });
        assert.deepEqual([r.isError, r.text], [true, `Refused: no space "${space}" that you can read.`], 'the same answer whether it exists or not');
      }
    } finally {
      await c.close();
    }
  });

  test('a call beyond the scopes is a 403 with insufficient_scope; beyond the grant, a refusal', async () => {
    const r = await request(server.port, { headers: bearer(reader), body: toolCall('context_log', { app: 'phone', space: 'project:atlas', title: 'x' }) });
    assert.equal(r.status, 403);
    assert.match(r.headers['www-authenticate'], /^Bearer error="insufficient_scope", error_description="[^"]+", scope="ctx:append", resource_metadata="http:\/\/127\.0\.0\.1:\d+\/\.well-known\/oauth-protected-resource\/mcp"$/);
    assert.match(textOf(r), /^Refused: this token's scopes don't include ctx:append\./);
    // The writer asks for ctx:write everywhere, but holds only append on its own space.
    const c = await httpClient(server.url, writer);
    try {
      assert.match((await call(c, 'context_log', { app: 'hermes', title: 'Backups checked' })).text, /^Saved note/, 'its own space is its default');
      assert.ok(fs.readdirSync(path.join(E.wikiDir, 'spaces', 'actor', 'hermes', 'inbox')).length === 1);
      const replace = await call(c, 'context_upsert_page', { app: 'hermes', space: 'actor:hermes', title: 'Backups', content: 'x', mode: 'replace' });
      assert.deepEqual([replace.isError, replace.text], [true, 'Refused: no space "actor:hermes" that you can write.']);
      const there = await call(c, 'context_log', { app: 'hermes', space: 'project:atlas', title: 'x' });
      assert.deepEqual([there.isError, there.text], [true, 'Refused: no space "project:atlas" that you can append.']);
      const start = await call(c, 'context_start', { app: 'hermes' });
      assert.match(start.text, /^Space: actor:hermes\. No other space is open to you\./m, 'append implies read');
    } finally {
      await c.close();
    }
  });

  test('without a token: this machine, unless "local" is "none"; a bad token is a 401', async () => {
    const plain = await request(server.port, { body: toolCall('context_search', { query: 'home lab' }) });
    assert.equal(plain.status, 200);
    assert.match(textOf(plain), /This machine has a home lab/);
    for (const [headers, error] of [[bearer(`xenolith_${'0'.repeat(64)}`), 'invalid_token'], [bearer('nonsense'), 'invalid_token'], [{ Authorization: 'Basic eDp5' }, 'invalid_request']]) {
      const r = await request(server.port, { headers, body: toolCall('context_search', { query: 'x' }) });
      assert.equal(r.status, 401, JSON.stringify(headers));
      assert.match(r.headers['www-authenticate'], new RegExp(`^Bearer error="${error}", error_description="[^"]+", resource_metadata="`));
      assert.equal(r.json.error, error);
    }
    const E2 = makeEnv({ access: { local: 'none' } });
    const s2 = await startHttp(E2.env);
    try {
      const r = await request(s2.port, { body: toolCall('context_search', { query: 'x' }) });
      assert.equal(r.status, 401);
      assert.match(r.headers['www-authenticate'], /^Bearer error_description="This server needs a token\.", resource_metadata="http:\/\/127\.0\.0\.1:\d+\/\.well-known\/oauth-protected-resource\/mcp"$/, 'no error code without credentials');
    } finally {
      s2.proc.kill();
    }
  });

  test('tokens only in the header, and only at /mcp', async () => {
    const inUrl = await request(server.port, { path: `/mcp?access_token=${reader}`, body: toolCall('context_search', { query: 'x' }) });
    assert.equal(inUrl.status, 400);
    assert.equal(inUrl.json.error, 'invalid_request');
    for (const p of ['/status', '/health', '/api/pages', '/api/held', '/ui/']) {
      const r = await request(server.port, { method: 'GET', path: p, headers: bearer(reader) });
      assert.equal(r.status, 403, p);
    }
    for (const p of ['/api/held', '/api/settings', '/api/forget']) {
      const r = await request(server.port, { path: p, headers: { ...bearer(reader), 'X-Xenolith': 'ui' }, body: { approvals: 'auto' } });
      assert.equal(r.status, 403, `POST ${p}: a token never decides anything in the window's API`);
    }
  });

  test("a read token reads content only: never the curator's held changes, history or locks", async () => {
    const atlas = path.join(E.wikiDir, 'spaces', 'project', 'atlas');
    fs.mkdirSync(path.join(atlas, '.curator', 'held'), { recursive: true });
    fs.writeFileSync(path.join(atlas, '.curator', 'held', '2026-10-03_10-00-00-000-bbbbbb.json'), '{"held":"a change waiting for the owner"}');
    fs.mkdirSync(path.join(atlas, '.history', 'pages', 'db'), { recursive: true });
    fs.writeFileSync(path.join(atlas, '.history', 'pages', 'db', 'old.md'), 'an older version');
    const c = await httpClient(server.url, reader);
    try {
      for (const target of ['.curator/held/2026-10-03_10-00-00-000-bbbbbb.json', '.curator/held', '.history/pages/db/old.md', '.locks']) {
        const r = await call(c, 'context_read', { space: 'project:atlas', target });
        assert.equal(r.isError, true, target);
        assert.match(r.text, /^Not found/, target);
      }
      const listing = await call(c, 'context_read', { space: 'project:atlas', target: '.' });
      assert.doesNotMatch(listing.text, /\.curator|\.history|\.locks/);
      assert.equal((await call(c, 'context_read', { space: 'project:atlas', target: 'index.md' })).isError, false);
      // The window's page view (with its .history list) is this machine's own, never a token's.
      assert.equal((await request(server.port, { method: 'GET', path: '/api/page?slug=db', headers: bearer(reader) })).status, 403);
    } finally {
      await c.close();
    }
  });

  test('a token reads machine:local only when its grant and scopes name it exactly', async () => {
    assert.equal(cli(E.env, 'grant', 'client:wide', 'ctx:read').code, 0);
    assert.equal(cli(E.env, 'grant', 'client:wide', 'ctx:read:machine:*').code, 0);
    const wide = JSON.parse(cli(E.env, 'token', 'create', 'client:wide', '--json').out).token;
    assert.equal(cli(E.env, 'grant', 'client:mine', 'ctx:read:machine:local').code, 0);
    const mine = JSON.parse(cli(E.env, 'token', 'create', 'client:mine', '--json').out).token;
    const search = (token, space, query) => request(server.port, { headers: bearer(token), body: toolCall('context_search', { space, query }) }).then(textOf);
    assert.equal(await search(wide, 'machine:local', 'home lab'), 'Refused: no space "machine:local" that you can read.', '"all" and machine:* do not reach it');
    assert.match(await search(wide, 'project:atlas', 'postgres'), /Atlas runs on Postgres 17/, '"all" still reaches the other spaces');
    assert.match(await search(mine, 'machine:local', 'home lab'), /This machine has a home lab/, 'a grant that names it');
  });

  test('a request a proxy forwarded is never this machine\'s own, whatever address it comes from', async () => {
    const plain = await request(server.port, { body: toolCall('context_search', { query: 'home lab' }) });
    assert.equal(plain.status, 200, 'loopback, no proxy headers: this machine');
    for (const via of [{ Forwarded: 'for=192.0.2.7;proto=https' }, { 'X-Forwarded-For': '192.0.2.7' }, { 'X-Forwarded-Host': 'studio.example' }, { Via: '1.1 gateway' }]) {
      const r = await request(server.port, { headers: via, body: toolCall('context_search', { query: 'home lab' }) });
      assert.equal(r.status, 401, JSON.stringify(via));
      assert.match(r.headers['www-authenticate'], /error_description="This server needs a token: a forwarded request is never this machine's own\."/);
      assert.doesNotMatch(r.text, /home lab/);
      for (const p of ['/status', '/api/pages', '/api/held', '/ui/']) {
        assert.equal((await request(server.port, { method: 'GET', path: p, headers: via })).status, 403, `${p} ${JSON.stringify(via)}`);
      }
    }
    // A token works through a proxy: it is what scopes remote access.
    const viaToken = await request(server.port, { headers: { ...bearer(reader), Forwarded: 'for=192.0.2.7' }, body: toolCall('context_search', { space: 'project:atlas', query: 'postgres' }) });
    assert.equal(viaToken.status, 200);
    assert.match(textOf(viaToken), /Atlas runs on Postgres 17/);
    // /health is open and says nothing private.
    const h = await request(server.port, { method: 'GET', path: '/health', headers: { Forwarded: 'for=192.0.2.7' } });
    assert.equal(h.status, 200);
    assert.equal(h.json.ok, true);
    assert.equal(h.json.wikiDir, undefined, 'no folder of this machine\'s');
    assert.ok(!h.text.includes(E.wikiDir) && !h.text.includes(E.home), h.text);
  });

  test('sync always needs a token, even from this machine', async () => {
    for (const [method, p] of [['GET', '/sync/v1/spaces/project/atlas/pages'], ['POST', '/sync/v1/spaces/project/atlas/entries']]) {
      const r = await request(server.port, { method, path: p, body: method === 'POST' ? { entries: [] } : undefined });
      assert.equal(r.status, 401, `${method} ${p}`);
      assert.match(r.headers['www-authenticate'], /^Bearer error_description="Sync needs a token, even from this machine\.", resource_metadata="/);
    }
  });

  test('20 bad tokens a minute from one source, then 429 for a minute; other sources and tokenless calls go on', async () => {
    const E4 = makeEnv();
    const s4 = await startHttp(E4.env);
    try {
      const bad = bearer(`xenolith_${'1'.repeat(64)}`);
      for (let i = 0; i < 20; i++) {
        assert.equal((await request(s4.port, { headers: bad, body: toolCall('context_search', { query: 'x' }) })).status, 401, `bad token ${i + 1}`);
      }
      const limited = await request(s4.port, { headers: bad, body: toolCall('context_search', { query: 'x' }) });
      assert.equal(limited.status, 429);
      assert.equal(limited.json.error, 'too_many_requests');
      assert.ok(Number(limited.headers['retry-after']) > 50, limited.headers['retry-after']);
      assert.equal((await request(s4.port, { method: 'GET', path: '/sync/v1/spaces/project/atlas/pages', headers: bad })).status, 429, 'sync too');
      // Behind a proxy, the client it names is its own source.
      assert.equal((await request(s4.port, { headers: { ...bad, Forwarded: 'for=192.0.2.9' }, body: toolCall('context_search', { query: 'x' }) })).status, 401);
      assert.equal((await request(s4.port, { body: toolCall('context_search', { query: 'x' }) })).status, 200, 'this machine without a token is not a token guesser');
      const chain = fs.readFileSync(path.join(E4.home, 'audit', 'chain.jsonl'), 'utf8').trim().split('\n').map((l) => JSON.parse(l));
      assert.equal(chain.filter((l) => l.why === 'too many bad tokens').length, 1, 'the block is in the audit chain, once');
    } finally {
      s4.proc.kill();
    }
  });

  test('the resource metadata a forwarded caller sees follows config.json\'s publicUrl', async () => {
    const E5 = makeEnv();
    fs.writeFileSync(path.join(E5.home, 'config.json'), JSON.stringify({ writeMode: 'curated', publicUrl: 'https://studio.example/xenolith/' }));
    const s5 = await startHttp(E5.env);
    try {
      const via = { Forwarded: 'for=192.0.2.7;proto=https' };
      const remote = await request(s5.port, { method: 'GET', path: '/.well-known/oauth-protected-resource/xenolith/mcp', headers: via });
      assert.equal(remote.status, 200);
      assert.equal(remote.json.resource, 'https://studio.example/xenolith/mcp');
      const r = await request(s5.port, { headers: via, body: toolCall('context_search', { query: 'x' }) });
      assert.equal(r.status, 401);
      assert.match(r.headers['www-authenticate'], /resource_metadata="https:\/\/studio\.example\/\.well-known\/oauth-protected-resource\/xenolith\/mcp"$/);
      const here = await request(s5.port, { method: 'GET', path: '/.well-known/oauth-protected-resource' });
      assert.equal(here.json.resource, `http://127.0.0.1:${s5.port}/mcp`, 'this machine sees the address it used');
    } finally {
      s5.proc.kill();
    }
  });

  test('an audit chain that cannot be written is never silent: /status and `xenolith status` say so', async () => {
    const E6 = makeEnv();
    fs.mkdirSync(path.join(E6.home, 'audit', 'chain.jsonl'), { recursive: true }); // a folder where the chain goes
    const s6 = await startHttp(E6.env);
    try {
      await request(s6.port, { headers: bearer(`xenolith_${'2'.repeat(64)}`), body: toolCall('context_search', { query: 'x' }) });
      const st = (await request(s6.port, { method: 'GET', path: '/status' })).json;
      assert.equal(st.audit.ok, false);
      assert.equal(st.audit.writable, false);
      assert.ok(st.audit.failures >= 1);
      assert.equal(st.health, 'degraded');
      assert.ok(st.reasons.some((x) => /^the audit chain can't be written: /.test(x)), JSON.stringify(st.reasons));
      const out = cli(E6.env, 'status', '--port', String(s6.port));
      assert.equal(out.code, 1);
      assert.match(out.out, /^Audit chain: ! /m);
      assert.match(out.out, /^  ! the audit chain can't be written/m);
      fs.rmSync(path.join(E6.home, 'audit', 'chain.jsonl'), { recursive: true });
      const fine = cli(E6.env, 'status', '--port', String(s6.port), '--json');
      assert.equal(JSON.parse(fine.out).audit.writable, true);
    } finally {
      s6.proc.kill();
    }
  });

  test('an unknown "local" value in access.json means none', async () => {
    const E3 = makeEnv({ access: { local: 'everything' } });
    const s3 = await startHttp(E3.env);
    try {
      const r = await request(s3.port, { body: toolCall('context_search', { query: 'x' }) });
      assert.equal(r.status, 401);
      // `xenolith access local` sets it in one line, and the service reads it on the next request.
      assert.deepEqual(JSON.parse(cli(E3.env, 'access', 'local', 'read', '--json').out), { local: 'read' });
      assert.equal((await request(s3.port, { body: toolCall('context_search', { query: 'x' }) })).status, 200);
      const write = await request(s3.port, { body: toolCall('context_log', { app: 'x', title: 'x' }) });
      assert.match(textOf(write), /^Refused: /);
      assert.equal(cli(E3.env, 'access', 'local', 'everything').code, 1);
      // The window's API follows it too: "read" reads, but pausing the curator needs admin or the owner key.
      const owner = { 'X-Xenolith': 'ui', 'X-Xenolith-Owner': JSON.parse(fs.readFileSync(path.join(E3.home, 'runtime', 'owner.key'), 'utf8')).key };
      assert.equal((await request(s3.port, { method: 'GET', path: '/api/pages' })).status, 200);
      const pause = await request(s3.port, { path: '/api/curator', headers: { 'X-Xenolith': 'ui' }, body: { paused: true } });
      assert.equal(pause.status, 403);
      assert.match(pause.json.error, /needs admin access for this machine's processes \(access\.json says read\), or the owner key/);
      assert.equal((await request(s3.port, { path: '/api/curator', headers: owner, body: { paused: false } })).status, 200);
      assert.equal(cli(E3.env, 'access', 'local', 'none').code, 0);
      assert.equal((await request(s3.port, { body: toolCall('context_search', { query: 'x' }) })).status, 401);
      // "none": the window's API reads nothing without the key, and /status keeps the service's health only.
      assert.equal((await request(s3.port, { method: 'GET', path: '/api/pages' })).status, 403);
      assert.equal((await request(s3.port, { method: 'GET', path: '/api/pages', headers: owner })).status, 200);
      const bare = await request(s3.port, { method: 'GET', path: '/status' });
      assert.equal(bare.status, 200);
      assert.ok(bare.json.pid > 0 && bare.json.recent === undefined && bare.json.lastWrite === undefined, bare.text);
      assert.ok(Array.isArray((await request(s3.port, { method: 'GET', path: '/status', headers: owner })).json.recent), 'the owner sees all of it');
    } finally {
      s3.proc.kill();
    }
  });

  test('revoking a token, or its grant, takes effect on an open connection', async () => {
    assert.equal(cli(E.env, 'grant', 'client:temp', 'ctx:read:project:atlas').code, 0);
    const made = JSON.parse(cli(E.env, 'token', 'create', 'client:temp', '--json').out);
    const c = await httpClient(server.url, made.token);
    try {
      assert.equal((await call(c, 'context_search', { space: 'project:atlas', query: 'postgres' })).isError, false);
      assert.equal(cli(E.env, 'revoke', 'client:temp').code, 0);
      assert.match((await call(c, 'context_search', { space: 'project:atlas', query: 'postgres' })).text, /^Refused: no space "project:atlas"/, 'the grant is gone');
      assert.equal(cli(E.env, 'grant', 'client:temp', 'ctx:read:project:atlas').code, 0);
      assert.equal((await call(c, 'context_search', { space: 'project:atlas', query: 'postgres' })).isError, false);
      assert.deepEqual(JSON.parse(cli(E.env, 'token', 'revoke', made.id, '--json').out), { revoked: 1 });
      const r = await request(server.port, { headers: bearer(made.token), body: toolCall('context_search', { space: 'project:atlas', query: 'x' }) });
      assert.equal(r.status, 401, 'a revoked token is unknown');
    } finally {
      await c.close().catch(() => {});
    }
  });
});

describe('XENOLITH_TOKEN for a stdio server, and the token commands', { timeout: 60_000 }, () => {
  test('a stdio server started with a token works as its holder; an unknown one does not start', async () => {
    const E = makeEnv();
    assert.equal(cli(E.env, 'grant', 'actor:hermes', 'ctx:append:by:hermes').code, 0);
    const { token } = JSON.parse(cli(E.env, 'token', 'create', 'actor:hermes', '--json').out);
    const c = new Client({ name: 'hermes', version: '1' });
    await c.connect(new StdioClientTransport({ ...srvT(), env: { ...E.env, XENOLITH_TOKEN: token }, stderr: 'pipe' }));
    try {
      assert.match((await call(c, 'context_log', { app: 'hermes', title: 'Night run done' })).text, /^Saved note/);
      assert.match((await call(c, 'context_search', { space: 'machine:local', query: 'x' })).text, /^Refused: no space "machine:local" that you can read\.$/);
      assert.equal(cli(E.env, 'token', 'revoke', 'actor:hermes').code, 0);
      assert.match((await call(c, 'context_search', { query: 'night' })).text, /^Refused: this token has been revoked or has expired/);
    } finally {
      await c.close();
    }
    const bad = spawnSync(rustBin(), ['serve'], { env: { ...E.env, XENOLITH_TOKEN: token }, input: '', encoding: 'utf8' });
    assert.equal(bad.status, 1);
    assert.match(bad.stderr, /XENOLITH_TOKEN is not a token Xenolith knows/);
  });

  test("token create takes the subject's grants as its scopes, shows the token once, keeps only its hash", () => {
    const E = makeEnv();
    const none = cli(E.env, 'token', 'create', 'actor:hermes');
    assert.equal(none.code, 1);
    assert.match(none.err, /has no grants, so a token would reach nothing/);
    cli(E.env, 'grant', 'actor:hermes', 'ctx:append:by:hermes');
    cli(E.env, 'grant', 'actor:hermes', 'ctx:read:project:atlas');
    const plain = cli(E.env, 'token', 'create', 'actor:hermes', '--label', 'Hermes on the server', '--days', '7');
    assert.equal(plain.code, 0, plain.err);
    assert.match(plain.out, /^xenolith_[0-9a-f]{64}\n$/);
    assert.match(plain.err, /^Token tok_[0-9a-f]{12} for actor:hermes \(ctx:append:by:hermes ctx:read:project:atlas\), until \S+\. It is shown once/);
    const policy = JSON.parse(cli(E.env, 'access', '--json').out);
    assert.equal(policy.tokens.length, 1);
    assert.equal(policy.tokens[0].label, 'Hermes on the server');
    assert.equal(policy.tokens[0].hash, undefined, 'access --json shows ids, not hashes');
    assert.ok(!fs.readFileSync(path.join(E.home, 'access.json'), 'utf8').includes(plain.out.trim()), 'only the hash is kept');
    if (process.platform !== 'win32') assert.equal(fs.statSync(path.join(E.home, 'access.json')).mode & 0o777, 0o600, 'readable by you alone');
    assert.match(cli(E.env, 'token', 'create', 'actor:hermes', '--scope', 'ctx:read nope').err, /"nope" is not a scope/);
    assert.equal(cli(E.env, 'token', 'revoke', 'tok_000000000000').code, 1, 'nothing to revoke');
  });

  test('a token is a secret: the wiki refuses to store one', async () => {
    const E = makeEnv();
    const c = new Client({ name: 'x', version: '1' });
    await c.connect(new StdioClientTransport({ ...srvT(), env: E.env, stderr: 'pipe' }));
    try {
      const token = `xenolith_${'ab'.repeat(32)}`;
      const r = await call(c, 'context_log', { app: 'x', title: 'Token for hermes', body: `It is ${token}` });
      assert.equal(r.isError, true);
      assert.match(r.text, /^Refused: `body` looks like it contains a Xenolith or XenoNet token\./);
    } finally {
      await c.close();
    }
  });
});
