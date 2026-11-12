// Joining with an invite over xenolith/join/1 (the Rust programs only), as Xenolith instances on
// this one, each with its own temp home and folder and its own endpoint on 127.0.0.1: "studio"
// curates project:atlas and invites; "laptop" joins with the invite and its code; "desk" tries a
// used invite, wrong codes and no owner key, and gets nothing. Nothing here touches the installed
// Xenolith. crates/net/tests/join.rs checks what goes on the wire (no inviter proof to a wrong
// code, a proof caught by a fake inviter, an expired invite).

import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import dgram from 'node:dgram';
import fs from 'node:fs';
import fsp from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { after, before, describe, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { rustBin, startService } from './impl.mjs';

const repo = fileURLToPath(new URL('..', import.meta.url));
const fakeCodex = path.join(repo, 'test', 'fixtures', 'fake-codex.mjs');
const tmp = await fsp.mkdtemp(path.join(os.tmpdir(), 'xenolith-join-'));
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
  const config = { writeMode: 'curated', curator: { codexPath: fakeCodex, lint: 'off' }, sync: { bind: `127.0.0.1:${port}` } };
  fs.writeFileSync(path.join(home, 'config.json'), JSON.stringify(config));
  const env = { ...process.env, XENOLITH_DIR: path.join(dir, 'wiki'), XENOLITH_HOME: home };
  delete env.XENOLITH_TOKEN;
  delete env.XENOLITH_WRITE_MODE;
  const atlas = path.join(dir, 'wiki', 'spaces', 'project', 'atlas');
  return { name, dir, home, env, port, atlas, service: null };
}

function cli(m, args, input) {
  const r = spawnSync(rustBin(), args, { env: m.env, encoding: 'utf8', input: input ?? '' });
  return { code: r.status, out: r.stdout, err: r.stderr };
}

function ok(m, args, input) {
  const r = cli(m, args, input);
  assert.equal(r.code, 0, `${m.name}: xenolith ${args.join(' ')}: ${r.err}${r.out}`);
  return r;
}

const status = (m) => JSON.parse(ok(m, ['sync', 'status', '--json']).out).sync;

async function until(what, fn, ms = 20_000) {
  const t = Date.now();
  for (;;) {
    if (await fn()) return;
    if (Date.now() - t > ms) assert.fail(`${what}: not within ${ms} ms`);
    await new Promise((r) => setTimeout(r, 50));
  }
}

const page = (title, body) => `---\ntitle: ${title}\ntype: project\n---\n\n# ${title}\n\n${body}\n`;
const read = (f) => (fs.existsSync(f) ? fs.readFileSync(f, 'utf8') : null);
const followed = (m) => {
  const f = path.join(m.home, 'sync', 'follow.json');
  return fs.existsSync(f) ? JSON.parse(fs.readFileSync(f, 'utf8')).follows : [];
};

describe('join/1: an invite and its code', { timeout: 180_000 }, () => {
  let studio;
  let laptop;
  let desk;
  const invite = (level = 'write') => JSON.parse(ok(studio, ['invite', 'project:atlas', '--level', level, '--json']).out);
  const join = (m, inv, code) => cli(m, ['join', inv.invite, '--at', `127.0.0.1:${studio.port}`, '--json'], `${code}\n`);

  before(async () => {
    [studio, laptop, desk] = await Promise.all([machine('studio'), machine('laptop'), machine('desk')]);
    for (const m of [studio, laptop, desk]) ok(m, ['serve', '--init']);
    ok(studio, ['spaces', 'add', 'project:atlas']);
    fs.writeFileSync(path.join(studio.atlas, 'pages', 'db.md'), page('Atlas database', 'Postgres 16 on the studio box.'));
    for (const m of [studio, laptop, desk]) m.service = await startService(m.env);
    await until('the studio founded project:atlas', () => status(studio).spaces?.some((s) => s.address === 'project:atlas'));
  });
  after(async () => {
    for (const m of [studio, laptop, desk]) await m.service?.stop();
  });

  test('a Xenolith joins with the invite and its code, and the space syncs', async () => {
    const inv = invite();
    assert.match(inv.invite, /^xlj1_[A-Za-z0-9_-]+$/);
    assert.match(inv.code, /^([0-9A-HJKMNP-TV-Z]{4}-){6}[0-9A-HJKMNP-TV-Z]{2}$/, '26 characters, 130 bits');
    const space = status(studio).spaces.find((s) => s.address === 'project:atlas');
    assert.deepEqual(inv.offer.spaces.map((s) => [s.space, s.address]), [[space.id, 'project:atlas']]);
    assert.equal(inv.offer.curator, status(studio).key);
    // The code is never an argument.
    const asArg = cli(laptop, ['join', inv.invite, inv.code]);
    assert.equal(asArg.code, 2, asArg.err);
    const r = join(laptop, inv, inv.code.toLowerCase());
    assert.equal(r.code, 0, r.err);
    const out = JSON.parse(r.out);
    assert.deepEqual(out.followed.map((f) => [f.space, f.address, f.genesis]), [[space.id, 'project:atlas', inv.offer.spaces[0].genesis]]);
    assert.equal(out.level, 'write');
    const member = status(studio).spaces.find((s) => s.id === space.id).members.find((x) => x.key === status(laptop).key);
    assert.deepEqual([member.endpoint, member.level], [status(laptop).endpointId, 'write'], 'the Share names its context key and the endpoint that dialled');
    await until('the page reached the laptop', () => read(path.join(laptop.atlas, 'pages', 'db.md')) === read(path.join(studio.atlas, 'pages', 'db.md')));
  });

  test('a used invite is refused for a second join', () => {
    // The laptop's invite, used: the desk tries it with its code.
    const inv = invite();
    const first = join(laptop, inv, inv.code);
    assert.equal(first.code, 0, `used once: ${first.err}`);
    const r = join(desk, inv, inv.code);
    assert.notEqual(r.code, 0);
    assert.match(r.err, /used or retired|past its ten minutes/);
    assert.deepEqual(followed(desk), [], 'the desk follows nothing');
  });

  test('five wrong codes retire an invite, and none is answered with a proof', async () => {
    const inv = invite();
    const wrong = inv.code.replace(/^./, (c) => (c === 'A' ? 'B' : 'A'));
    for (let left = 4; left >= 1; left--) {
      const r = join(desk, inv, wrong);
      assert.notEqual(r.code, 0);
      assert.match(r.err, new RegExp(`doesn't match the invite: ${left} tries left \\(wrong-code\\)`));
      // A refusal comes before any proof of the inviter's: the joiner never got to check one.
      assert.doesNotMatch(r.err, /couldn't prove/);
    }
    const fifth = join(desk, inv, wrong);
    assert.match(fifth.err, /wrong five times, so the invite was retired/);
    const right = join(desk, inv, inv.code);
    assert.notEqual(right.code, 0, 'the right code after the fifth wrong one');
    assert.match(right.err, /used or retired/);
    assert.deepEqual(followed(desk), []);
    assert.ok(!fs.existsSync(desk.atlas), 'and has nothing of the space');
    await until("the studio's log says so", () => studio.service.log().includes('refused: wrong-code'));
    const members = status(studio).spaces.find((s) => s.address === 'project:atlas').members;
    assert.ok(!members.some((x) => x.key === status(desk).key), 'the desk was never shared with');
  });

  test('inviting and joining are refused without the owner key', async () => {
    for (const [m, route, body] of [
      [studio, '/api/sync/invite', { spaces: ['project:atlas'], level: 'write' }],
      [desk, '/api/sync/join', { invite: 'xlj1_x', code: 'x' }],
    ]) {
      const r = await fetch(`http://127.0.0.1:${m.service.port}${route}`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Xenolith': 'cli' }, body: JSON.stringify(body) });
      assert.equal(r.status, 403, `${route}: ${await r.text()}`);
    }
    const r = cli(studio, ['sync', 'share', 'project:atlas', 'xns1_x', 'f'.repeat(64)]);
    assert.equal(r.code, 2);
    assert.match(r.err, /gone; join with an invite/);
  });

  test('an invite names only spaces this Xenolith curates', () => {
    const r = cli(laptop, ['invite', 'project:atlas']);
    assert.notEqual(r.code, 0);
    assert.match(r.err, /curated by another Xenolith/);
  });
});
