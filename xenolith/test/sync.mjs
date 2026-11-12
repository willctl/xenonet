// Sync between machines over xenolith/sync/2 (the Rust programs only), as Xenolith instances on
// this one, each with its own temp home and folder and its own endpoint on 127.0.0.1: "studio"
// curates project:atlas and shares it with "laptop" (write) and "desk" (append), which follow it.
// Notes left on a member reach the curator's inbox, its curator files them, and the pages come
// back; edits in a member's copy go to the curator as proposals; a stranger and a member's stolen
// transport key get nothing. Nothing here touches the installed Xenolith.

import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import dgram from 'node:dgram';
import fs from 'node:fs';
import fsp from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { after, before, describe, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { curatorCmd, rustBin, srvT, startService } from './impl.mjs';

const repo = fileURLToPath(new URL('..', import.meta.url));
const fakeCodex = path.join(repo, 'test', 'fixtures', 'fake-codex.mjs');
const tmp = await fsp.mkdtemp(path.join(os.tmpdir(), 'xenolith-sync-'));
after(() => fsp.rm(tmp, { recursive: true, force: true, maxRetries: 5 }).catch(() => {}));

const freePort = () =>
  new Promise((resolve) => {
    const s = dgram.createSocket('udp4');
    s.bind(0, '127.0.0.1', () => {
      const p = s.address().port;
      s.close(() => resolve(p));
    });
  });

async function machine(name) {
  const dir = path.join(tmp, name);
  const home = path.join(dir, 'home');
  fs.mkdirSync(home, { recursive: true });
  const port = await freePort();
  const config = { writeMode: 'curated', curator: { codexPath: fakeCodex, lint: 'off', debounceSeconds: 0.2, maxWaitSeconds: 1, maxAttempts: 2, timeoutSeconds: 30 }, sync: { bind: `127.0.0.1:${port}` } };
  fs.writeFileSync(path.join(home, 'config.json'), JSON.stringify(config));
  const env = { ...process.env, XENOLITH_DIR: path.join(dir, 'wiki'), XENOLITH_HOME: home, FAKE_CODEX_LOG: path.join(dir, 'fake-codex.jsonl') };
  delete env.XENOLITH_TOKEN;
  delete env.XENOLITH_WRITE_MODE;
  const atlas = path.join(dir, 'wiki', 'spaces', 'project', 'atlas');
  return { name, dir, home, env, port, atlas, service: null };
}

function cli(m, ...args) {
  const r = spawnSync(rustBin(), args, { env: m.env, encoding: 'utf8' });
  return { code: r.status, out: r.stdout, err: r.stderr };
}

function ok(m, ...args) {
  const r = cli(m, ...args);
  assert.equal(r.code, 0, `${m.name}: xenolith ${args.join(' ')}: ${r.err}${r.out}`);
  return r;
}

const status = (m) => JSON.parse(ok(m, 'sync', 'status', '--json').out).sync;

async function start(m) {
  m.service = await startService(m.env);
}

async function stop(m) {
  await m.service?.stop();
  m.service = null;
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

function curate(m) {
  const r = spawnSync(...curatorCmd('--once'), { env: m.env, encoding: 'utf8' });
  assert.equal(r.status, 0, r.stderr);
}

async function until(what, fn, ms = 20_000) {
  const t = Date.now();
  for (;;) {
    if (await fn()) return Date.now() - t;
    if (Date.now() - t > ms) assert.fail(`${what}: not within ${ms} ms`);
    await new Promise((r) => setTimeout(r, 50));
  }
}

const md = (dir) => (fs.existsSync(dir) ? fs.readdirSync(dir).filter((f) => f.endsWith('.md')).map((f) => fs.readFileSync(path.join(dir, f), 'utf8')) : []);
const read = (f) => (fs.existsSync(f) ? fs.readFileSync(f, 'utf8') : null);
const page = (title, body) => `---\ntitle: ${title}\ntype: project\n---\n\n# ${title}\n\n${body}\n`;
const pageOf = (m, slug) => path.join(m.atlas, 'pages', `${slug}.md`);
const held = (m) => JSON.parse(ok(m, 'held', '--space', 'project:atlas', '--json').out).changes;

describe('sync/2 between Xenoliths', { timeout: 300_000 }, () => {
  let studio;
  let laptop;
  let desk;
  let space;
  const follow = (m) => ok(m, 'sync', 'follow', space, status(studio).key, status(studio).endpointId, `127.0.0.1:${studio.port}`);

  before(async () => {
    [studio, laptop, desk] = await Promise.all([machine('studio'), machine('laptop'), machine('desk')]);
    for (const m of [studio, laptop, desk]) ok(m, 'serve', '--init');
    ok(studio, 'spaces', 'add', 'project:atlas');
    fs.writeFileSync(pageOf(studio, 'db'), page('Atlas database', 'Postgres 16 on the studio box.'));
    await Promise.all([start(studio), start(laptop), start(desk)]);
    await until('the studio founded project:atlas', () => status(studio).spaces?.some((s) => s.address === 'project:atlas'));
    space = status(studio).spaces.find((s) => s.address === 'project:atlas').id;
    const shared = JSON.parse(ok(studio, 'sync', 'share', 'project:atlas', status(laptop).key, status(laptop).endpointId, '--level', 'write', '--json').out);
    assert.equal(shared.id, space);
    ok(studio, 'sync', 'share', 'project:atlas', status(desk).key, status(desk).endpointId, '--level', 'append');
    follow(laptop);
    follow(desk);
  });
  after(async () => {
    for (const m of [studio, laptop, desk]) await stop(m);
  });

  test("the curator's pages reach both members, each in a folder of its own, with nothing from 1.x", async () => {
    for (const m of [laptop, desk]) {
      await until(`${m.name} got the page`, () => read(pageOf(m, 'db')) === read(pageOf(studio, 'db')));
      const s = status(m).spaces.find((x) => x.id === space);
      assert.deepEqual([s.address, s.role, s.curator], ['project:atlas', 'member', status(studio).key]);
    }
    const members = status(studio).spaces.find((s) => s.id === space).members;
    assert.deepEqual(members.map((x) => [x.key, x.level]).sort(), [[status(laptop).key, 'write'], [status(desk).key, 'append']].sort());
    await until('both members are connected', () => status(studio).spaces.find((s) => s.id === space).members.every((x) => x.connected));
    assert.ok(!fs.existsSync(path.join(laptop.home, 'sync.json')), "2.0 never writes 1.x's sync.json");
  });

  test("a member's note reaches the curator's inbox once, its curator files it, and the page comes back", async () => {
    await session(laptop, (call) => call('context_log', { app: 'codex', space: 'project:atlas', title: 'Atlas moves to Postgres 17', body: 'Logical replication.', pages: ['db'] }));
    await until("the note reached the studio's inbox", () => md(path.join(studio.atlas, 'inbox')).some((n) => n.includes('Atlas moves to Postgres 17')));
    const n = md(path.join(studio.atlas, 'inbox')).find((x) => x.includes('Atlas moves to Postgres 17'));
    assert.match(n, /^transport: sync$/m);
    assert.match(n, new RegExp(`^by: "key:${status(laptop).key}"$`, 'm'));
    assert.match(n, /^source: agent$/m, "a standalone peer's note never speaks as the person");
    await until("it left the laptop's inbox", () => md(path.join(laptop.atlas, 'inbox')).length === 0);
    // The laptop's own curator leaves the copy alone: the studio's curator files it.
    curate(laptop);
    assert.equal(read(pageOf(laptop, 'db')), read(pageOf(studio, 'db')), "the laptop's curator didn't touch the copy");
    curate(studio);
    assert.equal(md(path.join(studio.atlas, 'inbox')).length, 0, 'filed');
    const filed = read(pageOf(studio, 'db'));
    assert.notEqual(filed, page('Atlas database', 'Postgres 16 on the studio box.'), "the curator's model changed the page");
    for (const m of [laptop, desk]) await until(`the filed page reached ${m.name}`, () => read(pageOf(m, 'db')) === filed);
  });

  test('an edit to a member page reaches the curator within 2 s, and a curator page arriving after it keeps it', async () => {
    fs.writeFileSync(pageOf(laptop, 'db'), page('Atlas database', 'Postgres 17, tuned on the laptop.'));
    const ms = await until('the edit reached the studio', () => md(path.join(studio.atlas, 'inbox')).some((n) => n.includes('tuned on the laptop')), 2_000);
    assert.ok(ms < 2_000, `${ms} ms`);
    const n = md(path.join(studio.atlas, 'inbox')).find((x) => x.includes('tuned on the laptop'));
    assert.match(n, /^page_mode: replace$/m);
    // The curator's own version arrives after it: the copy shows it, keeps the edit in .history/,
    // and the edit still waits for the curator.
    fs.writeFileSync(pageOf(studio, 'db'), page('Atlas database', 'Postgres 17, as the studio has it.'));
    await until("the studio's version reached the laptop", () => (read(pageOf(laptop, 'db')) ?? '').includes('as the studio has it'));
    const history = path.join(laptop.atlas, '.history', 'pages', 'db');
    assert.ok(fs.readdirSync(history).some((f) => fs.readFileSync(path.join(history, f), 'utf8').includes('tuned on the laptop')), 'the edit is kept in .history/');
    assert.ok(md(path.join(studio.atlas, 'inbox')).some((x) => x.includes('tuned on the laptop')), 'and not dropped on the studio');
    curate(studio);
  });

  test("a replace against an older version waits on the curator as a proposal, and the copy shows the curator's page", async () => {
    // The laptop is away while the studio's page changes, and a session there replaces the page
    // as the laptop has it: the note's base is a version the studio moved past.
    await stop(laptop);
    fs.writeFileSync(pageOf(studio, 'db'), page('Atlas database', 'Postgres 17 with a replica now.'));
    await session(laptop, (call) => call('context_upsert_page', { app: 'codex', space: 'project:atlas', title: 'Atlas database', slug: 'db', mode: 'replace', content: 'Postgres 18 on the laptop.' }));
    await start(laptop);
    await until('the stale replace is held on the studio', () => held(studio).some((h) => h.slug === 'db' && h.kind === 'sync'));
    const h = held(studio).find((x) => x.slug === 'db' && x.kind === 'sync');
    assert.match(h.reasons[0], new RegExp(`^key:${status(laptop).key} wrote this against version [0-9a-f]{16}, but the page is at [0-9a-f]{16} now\\.$`));
    await until("the laptop shows the studio's page", () => read(pageOf(laptop, 'db')) === read(pageOf(studio, 'db')));
    ok(studio, 'held', 'reject', `${h.batch}:0`, '--space', 'project:atlas');
  });

  test('a removal and the log reach the members; a member that may only append removes nothing', async () => {
    fs.writeFileSync(pageOf(studio, 'old'), page('Old', 'gone soon'));
    for (const m of [laptop, desk]) await until(`old reached ${m.name}`, () => fs.existsSync(pageOf(m, 'old')));
    fs.rmSync(pageOf(studio, 'old'));
    for (const m of [laptop, desk]) {
      await until(`old went from ${m.name}`, () => !fs.existsSync(pageOf(m, 'old')));
      assert.ok(fs.readdirSync(path.join(m.atlas, '.history', 'pages', 'old')).length >= 1, 'kept in .history/');
    }
    const day = new Date(Date.now() - new Date().getTimezoneOffset() * 60_000).toISOString().slice(0, 10);
    const logOf = (m) => path.join(m.atlas, 'log', day.slice(0, 4), `${day}.md`);
    await until("the curator's log reached the desk", () => (read(logOf(desk)) ?? '').includes('Removed') || (read(logOf(desk)) ?? '') === (read(logOf(studio)) ?? 'x'));
    // desk (append) removes a page in its copy: refused on the studio, and the page stays there.
    fs.rmSync(pageOf(desk, 'db'));
    await until('the studio said no', () => studio.service.log().includes(`wasn't taken: key:${status(desk).key} may not replace or remove pages`));
    assert.ok(fs.existsSync(pageOf(studio, 'db')));
  });

  test("a stranger is refused before its hello, and a member's transport key without its context key at its hello", async () => {
    const cafe = await machine('cafe');
    ok(cafe, 'serve', '--init');
    await start(cafe);
    follow(cafe);
    await new Promise((r) => setTimeout(r, 3_000));
    assert.ok(!fs.existsSync(cafe.atlas), 'the stranger got nothing');
    await stop(cafe);
    // thief: laptop's transport key, its own context key.
    const laptopEndpoint = status(laptop).endpointId;
    await stop(laptop);
    const thief = await machine('thief');
    ok(thief, 'serve', '--init');
    await start(thief);
    await stop(thief);
    fs.copyFileSync(path.join(laptop.home, 'keys', 'transport.key'), path.join(thief.home, 'keys', 'transport.key'));
    await start(thief);
    assert.equal(status(thief).endpointId, laptopEndpoint, "the laptop's endpoint id");
    assert.notEqual(status(thief).key, status(studio).spaces.find((s) => s.id === space).members.find((x) => x.level === 'write').key, 'and its own context key');
    follow(thief);
    await until("the studio refused the thief's hello", () => studio.service.log().includes('which no space here binds to that endpoint'));
    assert.ok(!fs.existsSync(thief.atlas), 'the thief got nothing');
    await stop(thief);
    await start(laptop);
  });

  test('a partition, then catch-up: what was written apart arrives once the two meet again', async () => {
    await until('the laptop is back', () => status(studio).spaces.find((s) => s.id === space).members.find((x) => x.key === status(laptop).key)?.connected);
    await stop(studio);
    await session(laptop, (call) => call('context_log', { app: 'codex', space: 'project:atlas', title: 'Written while apart', body: 'the studio was away' }));
    await stop(desk);
    fs.writeFileSync(pageOf(studio, 'apart'), page('Apart', 'made while the members were away'));
    await start(studio);
    await until('the note written apart reached the studio', () => md(path.join(studio.atlas, 'inbox')).some((n) => n.includes('Written while apart')), 45_000);
    await until('the page made apart reached the laptop', () => fs.existsSync(pageOf(laptop, 'apart')), 45_000);
    await start(desk);
    await until('and the desk, when it came back', () => fs.existsSync(pageOf(desk, 'apart')), 45_000);
    assert.equal(md(path.join(studio.atlas, 'inbox')).filter((n) => n.includes('Written while apart')).length, 1, 'once');
  });
});
