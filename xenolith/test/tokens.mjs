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
import { IMPL, rustBin, srv, srvT } from './impl.mjs';

const skip = IMPL !== 'rust' && 'the Rust programs only';
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
    const proc = spawn(...srv(runtime, '--http', '--port', '0', '--parent-stdin'), { env, stdio: ['pipe', 'pipe', 'pipe'] });
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

describe('tokens over HTTP', { skip, timeout: 60_000 }, () => {
  let E;
  let server;
  let reader; // ctx:read on project:atlas, granted read there
  let writer; // asks ctx:write everywhere, granted append on its own space only
  before(async () => {
    E = makeEnv();
    for (const [space, title] of [['project:atlas', 'Atlas runs on Postgres 17'], [null, 'This machine has a home lab']]) {
      const c = new Client({ name: 'setup', version: '1' });
      await c.connect(new StdioClientTransport({ ...srvT(runtime), env: E.env, stderr: 'pipe' }));
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
      assert.doesNotMatch(start.text, /Wiki folder:/);
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
    for (const p of ['/status', '/health', '/api/pages', '/ui/']) {
      const r = await request(server.port, { method: 'GET', path: p, headers: bearer(reader) });
      assert.equal(r.status, 403, p);
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

describe('XENOLITH_TOKEN for a stdio server, and the token commands', { skip, timeout: 60_000 }, () => {
  test('a stdio server started with a token works as its holder; an unknown one does not start', async () => {
    const E = makeEnv();
    assert.equal(cli(E.env, 'grant', 'actor:hermes', 'ctx:append:by:hermes').code, 0);
    const { token } = JSON.parse(cli(E.env, 'token', 'create', 'actor:hermes', '--json').out);
    const c = new Client({ name: 'hermes', version: '1' });
    await c.connect(new StdioClientTransport({ ...srvT(runtime), env: { ...E.env, XENOLITH_TOKEN: token }, stderr: 'pipe' }));
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
    assert.match(cli(E.env, 'token', 'create', 'actor:hermes', '--scope', 'ctx:read nope').err, /"nope" is not a scope/);
    assert.equal(cli(E.env, 'token', 'revoke', 'tok_000000000000').code, 1, 'nothing to revoke');
  });

  test('a token is a secret: the wiki refuses to store one', async () => {
    const E = makeEnv();
    const c = new Client({ name: 'x', version: '1' });
    await c.connect(new StdioClientTransport({ ...srvT(runtime), env: E.env, stderr: 'pipe' }));
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
