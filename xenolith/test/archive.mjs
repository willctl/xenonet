// The release archives' writers (scripts/archive.mjs): what they write must unpack with the system's
// own tools, the same way on every machine. scripts/verify-archive.mjs checks a real archive end to end.

import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { test } from 'node:test';
import { FIXED_TIME, crc32, entriesOf, tarGz, writeSums, zip } from '../scripts/archive.mjs';

const win = process.platform === 'win32';
const systemTar = win ? path.join(process.env.SystemRoot || 'C:\\Windows', 'System32', 'tar.exe') : 'tar';

function fixture() {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'xenolith-archive-test-'));
  const dir = path.join(tmp, 'xenolith-0.0.0-test');
  const deep = path.join('d'.repeat(60), `${'f'.repeat(60)}.txt`);
  const files = { xenolith: Buffer.from([0x7f, 0x45, 0x4c, 0x46, 0, 1, 2, 255]), 'README.md': 'read me\n', 'ui/index.html': '<!doctype html>\n', [deep]: 'a path past 100 bytes\n', 'ui/app.js': 'x'.repeat(5000) };
  for (const [rel, data] of Object.entries(files)) {
    fs.mkdirSync(path.dirname(path.join(dir, rel)), { recursive: true });
    fs.writeFileSync(path.join(dir, rel), data);
  }
  return { tmp, dir, files };
}

function unpacked(root, files) {
  for (const [rel, data] of Object.entries(files)) {
    assert.deepEqual(fs.readFileSync(path.join(root, 'xenolith-0.0.0-test', rel)), Buffer.from(data), rel);
  }
  if (!win) {
    assert.equal(fs.statSync(path.join(root, 'xenolith-0.0.0-test', 'xenolith')).mode & 0o777, 0o755);
    assert.equal(fs.statSync(path.join(root, 'xenolith-0.0.0-test', 'README.md')).mode & 0o777, 0o644);
  }
}

test('entries: folders first, names sorted, modes by name', () => {
  const { tmp, dir } = fixture();
  try {
    const e = entriesOf(dir, (rel) => rel.endsWith('/xenolith'));
    assert.deepEqual(
      e.map((x) => x.name),
      ['xenolith-0.0.0-test/', 'xenolith-0.0.0-test/README.md', `xenolith-0.0.0-test/${'d'.repeat(60)}/`, `xenolith-0.0.0-test/${'d'.repeat(60)}/${'f'.repeat(60)}.txt`, 'xenolith-0.0.0-test/ui/', 'xenolith-0.0.0-test/ui/app.js', 'xenolith-0.0.0-test/ui/index.html', 'xenolith-0.0.0-test/xenolith'],
    );
    assert.deepEqual(
      e.map((x) => x.mode.toString(8)),
      ['755', '644', '755', '644', '755', '644', '644', '755'],
    );
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
});

test('a .tar.gz is the same bytes every time and unpacks with tar', () => {
  const { tmp, dir, files } = fixture();
  try {
    const e = entriesOf(dir, (rel) => rel.endsWith('/xenolith'));
    const a = tarGz(e);
    assert.deepEqual(a, tarGz(entriesOf(dir, (rel) => rel.endsWith('/xenolith'))));
    const file = path.join(tmp, 'a.tar.gz');
    fs.writeFileSync(file, a);
    const out = path.join(tmp, 'tar');
    fs.mkdirSync(out);
    const r = spawnSync(systemTar, ['-xzf', file, '-C', out], { encoding: 'utf8' });
    assert.equal(r.status, 0, r.stderr);
    unpacked(out, files);
    assert.equal(Math.round(fs.statSync(path.join(out, 'xenolith-0.0.0-test', 'README.md')).mtimeMs / 1000), FIXED_TIME);
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
});

test('a .zip is the same bytes every time and unpacks with the system tools', (t) => {
  const { tmp, dir, files } = fixture();
  try {
    const e = entriesOf(dir, (rel) => rel.endsWith('/xenolith'));
    const a = zip(e);
    assert.deepEqual(a, zip(entriesOf(dir, (rel) => rel.endsWith('/xenolith'))));
    const file = path.join(tmp, 'a.zip');
    fs.writeFileSync(file, a);
    const out = path.join(tmp, 'zip');
    fs.mkdirSync(out);
    // bsdtar (macOS, Windows) reads a zip; GNU tar (Linux) does not, and unzip does.
    const r = process.platform === 'linux' ? spawnSync('unzip', ['-q', file, '-d', out], { encoding: 'utf8' }) : spawnSync(systemTar, ['-xf', file, '-C', out], { encoding: 'utf8' });
    if (r.error?.code === 'ENOENT') return t.skip('no unzip on this machine');
    assert.equal(r.status, 0, r.stderr);
    unpacked(out, files);
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
});

test('crc32 and SHA256SUMS', () => {
  assert.equal(crc32(Buffer.from('123456789')), 0xcbf43926);
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'xenolith-sums-'));
  try {
    fs.writeFileSync(path.join(tmp, 'xenolith-1.9.0-linux-x64.tar.gz'), 'a');
    fs.writeFileSync(path.join(tmp, 'xenolith-1.9.0-darwin-arm64.tar.gz'), 'b');
    fs.writeFileSync(path.join(tmp, 'notes.md'), 'not an archive');
    assert.deepEqual(writeSums(tmp), ['xenolith-1.9.0-darwin-arm64.tar.gz', 'xenolith-1.9.0-linux-x64.tar.gz']);
    assert.equal(
      fs.readFileSync(path.join(tmp, 'SHA256SUMS'), 'utf8'),
      '3e23e8160039594a33894f6564e1b1348bbd7a0088d42c4acb73eeaed59c009d  xenolith-1.9.0-darwin-arm64.tar.gz\nca978112ca1bbdcafac231b39a23dc4da786eff8147c4e72b9807785afee48bb  xenolith-1.9.0-linux-x64.tar.gz\n',
    );
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
});
