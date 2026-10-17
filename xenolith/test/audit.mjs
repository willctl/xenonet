// The audit chain (the Rust programs only): grants, tokens, refusals, writes and new spaces, one
// line each, each carrying the hash of the line before it, so `xenolith audit verify` finds a
// line that was changed, removed or put in between. It holds hashes of what sessions wrote,
// never the text.

import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import fsp from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { after, describe, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { IMPL, rustBin, srvT } from './impl.mjs';

const skip = IMPL !== 'rust' && 'the Rust programs only';
const repo = fileURLToPath(new URL('..', import.meta.url));
const runtime = path.join(repo, 'dist', 'runtime');
const tmp = await fsp.mkdtemp(path.join(os.tmpdir(), 'xenolith-audit-'));
after(() => fsp.rm(tmp, { recursive: true, force: true, maxRetries: 5 }).catch(() => {}));

function makeEnv() {
  const dir = fs.mkdtempSync(path.join(tmp, 'case-'));
  const home = path.join(dir, 'home');
  fs.mkdirSync(home, { recursive: true });
  fs.writeFileSync(path.join(home, 'config.json'), JSON.stringify({ writeMode: 'curated' }));
  const env = { ...process.env, XENOLITH_DIR: path.join(dir, 'wiki'), XENOLITH_HOME: home };
  delete env.XENOLITH_TOKEN;
  return { dir, home, env, chain: path.join(home, 'audit', 'chain.jsonl') };
}

function cli(env, ...args) {
  const r = spawnSync(rustBin(), args, { env, encoding: 'utf8' });
  return { code: r.status, out: r.stdout, err: r.stderr };
}

async function session(env, fn) {
  const c = new Client({ name: 'audit-test', version: '1' });
  await c.connect(new StdioClientTransport({ ...srvT(runtime), env, stderr: 'pipe' }));
  try {
    return await fn(async (name, args) => {
      const r = await c.callTool({ name, arguments: args });
      return { text: r.content.map((x) => x.text).join('\n'), isError: Boolean(r.isError) };
    });
  } finally {
    await c.close();
  }
}

const entries = (file) => fs.readFileSync(file, 'utf8').trim().split('\n').map((l) => JSON.parse(l));

describe('the audit chain', { skip, timeout: 60_000 }, () => {
  test('grants, tokens, writes, refusals and new spaces each leave one linked entry', async () => {
    const E = makeEnv();
    assert.equal(cli(E.env, 'grant', 'actor:hermes', 'ctx:append:by:hermes').code, 0);
    const { token, id } = JSON.parse(cli(E.env, 'token', 'create', 'actor:hermes', '--json').out);
    await session(E.env, async (call) => {
      assert.equal((await call('context_log', { app: 'codex', space: 'project:atlas', title: 'Atlas notes', body: 'The quiet words in this note.' })).isError, false);
    });
    await session({ ...E.env, XENOLITH_TOKEN: token }, async (call) => {
      assert.equal((await call('context_log', { app: 'hermes', title: 'Night run', body: 'Done.' })).isError, false);
      assert.equal((await call('context_search', { space: 'project:atlas', query: 'x' })).isError, true);
    });
    assert.equal(cli(E.env, 'token', 'revoke', id).code, 0);
    assert.equal(cli(E.env, 'revoke', 'actor:hermes').code, 0);

    const lines = entries(E.chain);
    assert.deepEqual(
      lines.map((l) => [l.kind, l.who]),
      [
        ['grant', 'you'],
        ['token', 'you'],
        ['space', 'you'],
        ['write', 'you'],
        ['write', `actor:hermes (${id})`],
        ['refused', `actor:hermes (${id})`],
        ['token-revoked', 'you'],
        ['revoke', 'you'],
      ],
    );
    assert.deepEqual(lines.map((l) => l.seq), [1, 2, 3, 4, 5, 6, 7, 8]);
    for (let i = 1; i < lines.length; i++) assert.equal(lines[i].prev, lines[i - 1].hash, `line ${i + 1} names the one before`);
    assert.equal(lines[0].subject, 'actor:hermes');
    assert.equal(lines[0].scope, 'ctx:append:by:hermes');
    assert.deepEqual([lines[2].space, lines[2].event], ['project:atlas', 'created']);
    assert.equal(lines[3].space, 'project:atlas');
    assert.equal(lines[3].tool, 'context_log');
    assert.match(lines[3].note, /^\d{4}-\d{2}-\d{2}_/);
    assert.match(lines[3].content, /^sha256:[0-9a-f]{32}$/);
    assert.equal(lines[4].space, 'actor:hermes');
    assert.deepEqual([lines[5].space, lines[5].need, lines[5].why], ['project:atlas', 'read', 'not granted']);
    const raw = fs.readFileSync(E.chain, 'utf8');
    for (const words of ['quiet words', 'Atlas notes', 'Night run', token]) assert.ok(!raw.includes(words), `no "${words}" in the chain`);

    const v = cli(E.env, 'audit', 'verify', '--json');
    assert.equal(v.code, 0, v.err);
    assert.deepEqual(JSON.parse(v.out), { lines: 8, head: lines.at(-1).hash, broken: null });
    assert.match(cli(E.env, 'audit').out, /^The audit chain holds: 8 entries, last hash [0-9a-f]{64}\.$/m);
    assert.deepEqual(JSON.parse(cli(E.env, 'audit', 'tail', '2', '--json').out).entries.map((l) => l.kind), ['token-revoked', 'revoke']);
    assert.match(cli(E.env, 'audit', 'tail').out, /^ +1 \S+ grant +you +subject=actor:hermes scope=ctx:append:by:hermes expiresAt=\d+$/m);
  });

  test('a changed, removed or added line breaks the chain there, and verify says where', () => {
    const E = makeEnv();
    for (const s of ['a', 'b', 'c']) assert.equal(cli(E.env, 'grant', `actor:${s}`, 'ctx:read').code, 0);
    const good = fs.readFileSync(E.chain, 'utf8');
    const lines = good.trim().split('\n');
    const broken = (text) => {
      fs.writeFileSync(E.chain, text);
      const r = cli(E.env, 'audit', 'verify', '--json');
      assert.equal(r.code, 1);
      return JSON.parse(r.out).broken;
    };
    assert.deepEqual(broken(good.replace('actor:b', 'actor:z')), { line: 2, why: 'its content changed after it was written' });
    assert.equal(broken(`${lines[0]}\n${lines[2]}\n`).line, 2, 'a removed line');
    assert.equal(broken(`${lines[0]}\n${lines[0]}\n${lines[1]}\n${lines[2]}\n`).line, 2, 'a repeated line');
    fs.writeFileSync(E.chain, good);
    assert.equal(cli(E.env, 'audit', 'verify').code, 0);
    assert.match(cli(E.env, 'audit', 'verify').out, /holds: 3 entries/);
  });
});
