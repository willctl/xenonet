// Spaces (the Rust programs only): one wiki each for a project, an identity, an actor or a
// machine, reached with the tools' `space` argument; who may use which, deny by default; and the
// commands that start spaces and grant them.

import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js';
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import fsp from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { after, describe, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { curatorCmd, rustBin, srvT, startService } from './impl.mjs';

const repo = fileURLToPath(new URL('..', import.meta.url));
const runtime = path.join(repo, 'dist', 'runtime');
const fakeCodex = path.join(repo, 'test', 'fixtures', 'fake-codex.mjs');
const tmp = await fsp.mkdtemp(path.join(os.tmpdir(), 'xenolith-spaces-'));
after(() => fsp.rm(tmp, { recursive: true, force: true, maxRetries: 5 }).catch(() => {}));

let n = 0;
/** A fresh wiki and home; `access` is written as the home's access.json. */
export function makeEnv({ access, config = {} } = {}) {
  const dir = path.join(tmp, `case-${++n}`);
  const wikiDir = path.join(dir, 'wiki');
  const home = path.join(dir, 'home');
  fs.mkdirSync(home, { recursive: true });
  fs.writeFileSync(path.join(home, 'config.json'), JSON.stringify({ writeMode: 'curated', curator: { codexPath: fakeCodex, lint: 'off', debounceSeconds: 0.2, maxWaitSeconds: 1, maxAttempts: 2, timeoutSeconds: 30 }, ...config }));
  if (access) fs.writeFileSync(path.join(home, 'access.json'), JSON.stringify(access));
  const env = { ...process.env, XENOLITH_DIR: wikiDir, XENOLITH_HOME: home, FAKE_CODEX_LOG: path.join(dir, 'fake-codex.jsonl') };
  delete env.XENOLITH_WRITE_MODE;
  delete env.XENOLITH_TOKEN;
  return { dir, wikiDir, home, env, logs: path.join(home, 'logs') };
}

async function connect(env) {
  const client = new Client({ name: 'spaces-test', version: '1.0.0' });
  await client.connect(new StdioClientTransport({ ...srvT(), env, stderr: 'pipe' }));
  return client;
}

async function call(client, name, args) {
  const r = await client.callTool({ name, arguments: args });
  return { text: r.content.map((c) => c.text).join('\n'), isError: Boolean(r.isError) };
}

async function ok(client, name, args) {
  const r = await call(client, name, args);
  assert.equal(r.isError, false, `${name} failed: ${r.text}`);
  return r.text;
}

async function refused(client, name, args) {
  const r = await call(client, name, args);
  assert.equal(r.isError, true, `${name} should have been refused: ${r.text}`);
  return r.text;
}

/** Runs `xenolith <args>`; returns {code, out, err}. */
function cli(env, ...args) {
  const r = spawnSync(rustBin(), args, { env, encoding: 'utf8' });
  return { code: r.status, out: r.stdout, err: r.stderr };
}

const files = (dir) => (fs.existsSync(dir) ? fs.readdirSync(dir).filter((f) => f.endsWith('.md')) : []);

describe('spaces', { timeout: 60_000 }, () => {
  test("a note logged to project:atlas starts that space and stays out of this machine's", async () => {
    const E = makeEnv();
    const c = await connect(E.env);
    try {
      const saved = await ok(c, 'context_log', { app: 'claude-code', space: 'project:atlas', title: 'Atlas moved to Postgres 17', body: 'For logical replication.' });
      assert.match(saved, /^Saved note \S+ \(inbox\/\S+\.md\)/);
      const atlas = path.join(E.wikiDir, 'spaces', 'project', 'atlas');
      assert.equal(files(path.join(atlas, 'inbox')).length, 1, "the note is in the space's own inbox");
      assert.ok(fs.statSync(path.join(atlas, 'PROTOCOL.md')).isFile(), 'the space has the whole wiki layout');
      assert.equal(files(path.join(E.wikiDir, 'inbox')).length, 0, "nothing reached this machine's inbox");
      assert.match(await ok(c, 'context_search', { space: 'project:atlas', query: 'postgres replication' }), /Atlas moved to Postgres 17/);
      assert.match(await ok(c, 'context_search', { query: 'postgres replication' }), /^No results/, "machine:local doesn't see it");
      assert.match(await ok(c, 'context_search', { space: 'machine:local', query: 'postgres replication' }), /^No results/);
    } finally {
      await c.close();
    }
  });

  test('context_start names the space and the others you can use; old tool names take a space too', async () => {
    const E = makeEnv();
    const c = await connect(E.env);
    try {
      const plain = await ok(c, 'context_start', { app: 'codex' });
      assert.doesNotMatch(plain, /^Space:/m, 'one space: nothing about spaces');
      await ok(c, 'context_log', { app: 'codex', space: 'actor:hermes', title: 'Hermes checked the backups', body: 'All green.' });
      const start = await ok(c, 'context_start', { app: 'codex', space: 'actor:hermes' });
      assert.match(start, /^Space: actor:hermes\. Other spaces you can use: machine:local \(admin\)\. Pass `space` to any tool to work in one\.$/m);
      assert.match(start, /Hermes checked the backups/, 'its pending notes');
      assert.match(await ok(c, 'context_start', { app: 'codex' }), /^Space: machine:local\. Other spaces you can use: actor:hermes \(admin\)\./m);
      assert.match(await ok(c, 'wiki_search', { space: 'actor:hermes', query: 'backups' }), /Hermes checked the backups/);
    } finally {
      await c.close();
    }
  });

  test("this machine's space can't read into spaces/ by path, nor list it", async () => {
    const E = makeEnv();
    const c = await connect(E.env);
    try {
      await ok(c, 'context_log', { app: 'codex', space: 'project:atlas', title: 'Secret plan', body: 'Only for atlas.' });
      const note = files(path.join(E.wikiDir, 'spaces', 'project', 'atlas', 'inbox'))[0];
      for (const target of [`spaces/project/atlas/inbox/${note}`, 'spaces', 'Spaces/project', 'pages/../spaces/project/atlas/PROTOCOL.md', './spaces/project/atlas']) {
        assert.match(await refused(c, 'context_read', { target }), /^Not found/, target);
      }
      assert.doesNotMatch(await ok(c, 'context_read', { target: '.' }), /spaces\//, 'the root listing leaves spaces/ out');
      assert.match(await ok(c, 'context_read', { space: 'project:atlas', target: `inbox/${note}` }), /Only for atlas\./, 'read in its own space');
    } finally {
      await c.close();
    }
  });

  test('a bad space, or one that does not exist yet, gets a plain answer', async () => {
    const E = makeEnv();
    const c = await connect(E.env);
    try {
      assert.match(await refused(c, 'context_search', { space: 'atlas', query: 'x' }), /is not a space: use project:<name>/);
      assert.match(await refused(c, 'context_search', { space: 'app:codex', query: 'x' }), /is not a kind of space/);
      assert.match(await refused(c, 'context_log', { space: 'project:../x', app: 'a', title: 't' }), /is not a space name/);
      assert.match(await refused(c, 'context_start', { app: 'a', space: 'project:nope' }), /^No space "project:nope" yet\. Logging a note to it starts it\./);
      assert.ok(!fs.existsSync(path.join(E.wikiDir, 'spaces', 'project', 'nope')), 'a read starts nothing');
    } finally {
      await c.close();
    }
  });

  test('with "local": "none", even this machine gets nothing', async () => {
    const E = makeEnv({ access: { local: 'none' } });
    const c = await connect(E.env);
    try {
      assert.match(await refused(c, 'context_start', { app: 'a' }), /you may not read in any space/);
      assert.match(await refused(c, 'context_log', { app: 'a', space: 'project:atlas', title: 't' }), /^Refused: no space "project:atlas" that you can append\./);
    } finally {
      await c.close();
    }
  });

  test("each space has its own curator: a space's notes become that space's pages", async () => {
    const E = makeEnv();
    const c = await connect(E.env);
    try {
      await ok(c, 'context_log', { app: 'codex', space: 'project:atlas', title: 'Atlas uses Postgres 17', body: 'Chosen for logical replication.', pages: ['atlas-db'] });
      await ok(c, 'context_log', { app: 'codex', title: 'Home lab has a new switch', body: 'A 2.5 GbE one.', pages: ['home-lab'] });
    } finally {
      await c.close();
    }
    const run = spawn(...curatorCmd('--once'), { env: E.env, stdio: ['ignore', 'pipe', 'pipe'] });
    let err = '';
    run.stderr.on('data', (d) => (err += d));
    const code = await new Promise((r) => run.on('exit', r));
    assert.equal(code, 0, err);
    const atlas = path.join(E.wikiDir, 'spaces', 'project', 'atlas');
    assert.deepEqual(files(path.join(atlas, 'pages')), ['atlas-db.md']);
    assert.deepEqual(files(path.join(E.wikiDir, 'pages')), ['home-lab.md']);
    assert.equal(files(path.join(atlas, 'inbox')).length, 0, "the space's inbox is filed");
  });
});

describe('spaces and grants from the command line', { timeout: 60_000 }, () => {
  test('spaces add, with the identities a project belongs to', () => {
    const E = makeEnv();
    assert.equal(cli(E.env, 'serve', '--init').code, 0);
    const added = cli(E.env, 'spaces', 'add', 'project:Atlas', '--identity', 'Adal', '--json');
    assert.equal(added.code, 0, added.err);
    assert.deepEqual(JSON.parse(added.out), { space: 'project:atlas', identities: ['adal'] });
    const list = JSON.parse(cli(E.env, 'spaces', '--json').out).spaces;
    assert.deepEqual(list.map((s) => s.space), ['machine:local', 'project:atlas']);
    assert.match(cli(E.env, 'spaces', 'add', 'identity:adal', '--identity', 'x').err, /--identity is for project spaces/);
    assert.match(cli(E.env, 'spaces', 'add', 'nope').err, /is not a space/);
  });

  test("grant, revoke and access; a grant that names one actor's space starts it", () => {
    const E = makeEnv();
    const g = cli(E.env, 'grant', 'actor:hermes', 'ctx:append:by:hermes', '--days', '2', '--json');
    assert.equal(g.code, 0, g.err);
    const grant = JSON.parse(g.out);
    assert.equal(grant.scope, 'ctx:append:by:hermes');
    assert.ok(Math.abs(grant.expiresAt - (Date.now() + 2 * 86_400_000)) < 60_000, 'two days');
    assert.ok(fs.statSync(path.join(E.wikiDir, 'spaces', 'actor', 'hermes', 'pages')).isDirectory(), "the actor's space is ready for it");
    assert.equal(cli(E.env, 'grant', 'client:phone', 'ctx:read:identity:adal', '--json').code, 0);
    const policy = JSON.parse(cli(E.env, 'access', '--json').out);
    assert.equal(policy.local, 'admin');
    assert.deepEqual(policy.grants.map((x) => [x.subject, x.scope]), [['actor:hermes', 'ctx:append:by:hermes'], ['client:phone', 'ctx:read:identity:adal']]);
    assert.match(cli(E.env, 'access').out, /grant {2}actor:hermes +ctx:append:by:hermes +until /);
    assert.match(cli(E.env, 'grant', 'hermes', 'ctx:read').err, /is not a subject/);
    assert.match(cli(E.env, 'grant', 'actor:hermes', 'read').err, /is not a scope/);
    assert.match(cli(E.env, 'grant', 'actor:hermes', 'ctx:read:app:codex').err, /is not a scope/, 'session keys select no space');
    assert.deepEqual(JSON.parse(cli(E.env, 'revoke', 'actor:hermes', '--json').out), { revoked: 1 });
    assert.deepEqual(JSON.parse(cli(E.env, 'access', '--json').out).grants.map((x) => x.subject), ['client:phone']);
    if (process.platform !== 'win32') assert.equal(fs.statSync(path.join(E.home, 'access.json')).mode & 0o777, 0o600, 'only you can read the policy');
  });

  test('a policy that cannot be read is never changed, and says why', () => {
    const E = makeEnv();
    fs.writeFileSync(path.join(E.home, 'access.json'), '{ broken');
    const g = cli(E.env, 'grant', 'actor:hermes', 'ctx:read');
    assert.equal(g.code, 1);
    assert.match(g.err, /Not changed: .*access\.json is not a JSON object/);
    assert.equal(fs.readFileSync(path.join(E.home, 'access.json'), 'utf8'), '{ broken');
    assert.match(JSON.parse(cli(E.env, 'access', '--json').out).problem, /not a JSON object/);
  });
});

const stopService = (s) => s.stop();

const pause = (ms) => new Promise((r) => setTimeout(r, ms));

/** A space's chain as `xenolith spaces --json` shows it, once the service has founded it. */
async function chainOf(env, space) {
  for (let i = 0; i < 100; i++) {
    const s = JSON.parse(cli(env, 'spaces', '--json').out).spaces.find((x) => x.space === space);
    if (s?.chain) return s.chain;
    await pause(100);
  }
  throw new Error(`${space} has no chain`);
}

const markerOf = (E, ...rel) => path.join(E.wikiDir, ...rel, 'xenolith-space.json');
const readMarker = (file) => JSON.parse(fs.readFileSync(file, 'utf8'));

describe('a chain per space', { timeout: 60_000 }, () => {
  test('the service founds one for each space it curates, and a hand-edited marker changes nothing', async () => {
    const E = makeEnv();
    assert.equal(cli(E.env, 'spaces', 'add', 'project:atlas').code, 0);
    assert.equal(JSON.parse(cli(E.env, 'spaces', '--json').out).spaces[1].chain, null, 'none until the service starts');
    const atlasMarker = markerOf(E, 'spaces', 'project', 'atlas');
    let s = await startService(E.env);
    let atlas;
    try {
      atlas = await chainOf(E.env, 'project:atlas');
      assert.match(atlas.id, /^spc_[a-z2-7]{32}$/);
      assert.deepEqual([atlas.about, atlas.weaker, atlas.epoch], ['name:project:atlas', true, 0], 'founded under its name, owned by the standalone key');
      assert.match(atlas.owner, /^key:xns1_/);
      const local = await chainOf(E.env, 'machine:local');
      assert.equal(readMarker(markerOf(E)).space, local.id, "this machine's own space too");
      const marker = readMarker(atlasMarker);
      assert.deepEqual([marker.space, marker.about, marker.owner], [atlas.id, 'name:project:atlas', atlas.owner]);
      // A space a stdio server starts while the service runs is founded within seconds.
      const c = await connect(E.env);
      try {
        await ok(c, 'context_log', { app: 'claude-code', space: 'project:zephyr', title: 'Zephyr starts', body: 'A new project.' });
      } finally {
        await c.close();
      }
      const zephyr = await chainOf(E.env, 'project:zephyr');
      assert.notEqual(zephyr.id, atlas.id);
      // By hand: atlas's marker now names zephyr, and a person as its owner.
      fs.writeFileSync(atlasMarker, JSON.stringify({ ...marker, space: zephyr.id, owner: `person:prs_${'a'.repeat(32)}` }));
    } finally {
      await stopService(s);
    }
    s = await startService(E.env);
    try {
      assert.deepEqual(await chainOf(E.env, 'project:atlas'), atlas, 'the same chain: the marker decided nothing');
      for (let i = 0; i < 50 && readMarker(atlasMarker).space !== atlas.id; i++) await pause(100);
      assert.equal(readMarker(atlasMarker).space, atlas.id, 'and the marker is written again');
      const c = await connect(E.env);
      try {
        await ok(c, 'context_log', { app: 'claude-code', space: 'project:atlas', title: 'Atlas still', body: 'Notes go where they went.' });
      } finally {
        await c.close();
      }
      assert.equal(files(path.join(E.wikiDir, 'spaces', 'project', 'atlas', 'inbox')).length, 1);
      assert.equal(files(path.join(E.wikiDir, 'spaces', 'project', 'zephyr', 'inbox')).length, 1, "nothing more reached zephyr's");
    } finally {
      await stopService(s);
    }
  });

  test('spaces bind: a dry run says what it would do; --run asks the service, whose chain refuses a space the owner key owns', async () => {
    const E = makeEnv();
    assert.equal(cli(E.env, 'spaces', 'add', 'project:atlas').code, 0);
    const s = await startService(E.env);
    try {
      const atlas = await chainOf(E.env, 'project:atlas');
      const prj = `prj_${'b'.repeat(32)}`;
      const dry = cli(E.env, 'spaces', 'bind', 'project:atlas', '--project', prj, '--repo', 'git@github.com:willctl/atlas.git', '--json');
      assert.equal(dry.code, 0, dry.err);
      const { dryRun, plan } = JSON.parse(dry.out);
      assert.equal(dryRun, true);
      assert.deepEqual([plan.id, plan.about, plan.to, plan.repo, plan.canRun], [atlas.id, 'name:project:atlas', prj, 'github.com/willctl/atlas', false]);
      assert.match(plan.why, /moves to the person/);
      assert.match(cli(E.env, 'spaces', 'bind', 'project:atlas', '--repo', 'https://github.com/willctl/atlas').out, /--project prj_.*Nothing was changed: this was a dry run\./s);
      const run = cli(E.env, 'spaces', 'bind', 'project:atlas', '--project', prj, '--run');
      assert.equal(run.code, 1);
      assert.match(run.err, /not bound: .*move-owner/);
      assert.deepEqual(await chainOf(E.env, 'project:atlas'), atlas, 'the chain is as it was');
      assert.match(cli(E.env, 'spaces', 'bind', 'project:atlas', '--project', 'nope').err, /not a project's id/);
    } finally {
      await stopService(s);
    }
  });
});
