// Release archives without dependencies: a .tar.gz (macOS, Linux) or a .zip (Windows) of one folder,
// and the SHA256SUMS beside them. The same folder gives the same bytes on any machine: entries in
// sorted order, one fixed time, modes set by name (programs 0755, everything else 0644), no owner
// names. The time is SOURCE_DATE_EPOCH when set, else 1980-01-01, the earliest a zip can hold.

import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import zlib from 'node:zlib';

export const FIXED_TIME = Number.parseInt(process.env.SOURCE_DATE_EPOCH ?? '', 10) || 315532800;

/**
 * The folder `dir` as archive entries under its own name: each folder before what it holds, names
 * sorted bytewise, '/' between parts. `executable(rel)` says which files get 0755.
 */
export function entriesOf(dir, executable) {
  const out = [];
  const walk = (abs, rel) => {
    out.push({ name: `${rel}/`, dir: true, mode: 0o755 });
    const names = fs.readdirSync(abs).sort((a, b) => Buffer.compare(Buffer.from(a), Buffer.from(b)));
    for (const n of names) {
      const a = path.join(abs, n);
      const r = `${rel}/${n}`;
      const st = fs.lstatSync(a);
      if (st.isSymbolicLink()) throw new Error(`${r}: a release archive holds no links`);
      if (st.isDirectory()) walk(a, r);
      else out.push({ name: r, dir: false, mode: executable(r) ? 0o755 : 0o644, data: fs.readFileSync(a) });
    }
  };
  walk(dir, path.basename(dir));
  return out;
}

// ---------------------------------------------------------------- tar (ustar) in gzip

function octal(buf, at, width, n) {
  buf.write(`${n.toString(8).padStart(width - 1, '0')}\0`, at, width, 'ascii');
}

function tarHeader(e, mtime) {
  const h = Buffer.alloc(512);
  let name = e.name;
  let prefix = '';
  if (Buffer.byteLength(name) > 100) {
    // ustar splits a long path at a '/' into a prefix (155 bytes) and a name (100).
    const cut = name.lastIndexOf('/', name.length - 2);
    prefix = name.slice(0, cut);
    name = name.slice(cut + 1);
    if (cut < 0 || Buffer.byteLength(name) > 100 || Buffer.byteLength(prefix) > 155) throw new Error(`${e.name}: path too long for a tar entry`);
  }
  h.write(name, 0, 100, 'utf8');
  octal(h, 100, 8, e.mode);
  octal(h, 108, 8, 0);
  octal(h, 116, 8, 0);
  octal(h, 124, 12, e.dir ? 0 : e.data.length);
  octal(h, 136, 12, mtime);
  h.fill(0x20, 148, 156);
  h.write(e.dir ? '5' : '0', 156, 1, 'ascii');
  h.write('ustar\0', 257, 6, 'ascii');
  h.write('00', 263, 2, 'ascii');
  octal(h, 329, 8, 0);
  octal(h, 337, 8, 0);
  h.write(prefix, 345, 155, 'utf8');
  let sum = 0;
  for (const b of h) sum += b;
  h.write(`${sum.toString(8).padStart(6, '0')}\0 `, 148, 8, 'ascii');
  return h;
}

export function tarGz(entries, mtime = FIXED_TIME) {
  const parts = [];
  for (const e of entries) {
    parts.push(tarHeader(e, mtime));
    if (!e.dir) {
      parts.push(e.data);
      const pad = (512 - (e.data.length % 512)) % 512;
      if (pad) parts.push(Buffer.alloc(pad));
    }
  }
  parts.push(Buffer.alloc(1024));
  const gz = zlib.gzipSync(Buffer.concat(parts), { level: 9 });
  // The gzip header names the OS zlib was built for; one value keeps the bytes the same everywhere.
  gz[9] = 3;
  return gz;
}

// ---------------------------------------------------------------- zip

const CRC_TABLE = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});

export function crc32(buf) {
  let c = 0xffffffff;
  for (const b of buf) c = CRC_TABLE[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

/** MS-DOS date and time, as a zip stores them: local time without a zone, so the UTC fields are used. */
function dosTime(unix) {
  const d = new Date(Math.max(unix, 315532800) * 1000);
  return {
    time: (d.getUTCHours() << 11) | (d.getUTCMinutes() << 5) | (d.getUTCSeconds() >> 1),
    date: ((d.getUTCFullYear() - 1980) << 9) | ((d.getUTCMonth() + 1) << 5) | d.getUTCDate(),
  };
}

export function zip(entries, mtime = FIXED_TIME) {
  const { time, date } = dosTime(mtime);
  const locals = [];
  const central = [];
  let offset = 0;
  for (const e of entries) {
    const name = Buffer.from(e.name, 'utf8');
    const raw = e.dir ? Buffer.alloc(0) : e.data;
    const data = e.dir ? raw : zlib.deflateRawSync(raw, { level: 9 });
    const method = e.dir ? 0 : 8;
    const crc = crc32(raw);
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt16LE(0x0800, 6); // names are UTF-8
    local.writeUInt16LE(method, 8);
    local.writeUInt16LE(time, 10);
    local.writeUInt16LE(date, 12);
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(data.length, 18);
    local.writeUInt32LE(raw.length, 22);
    local.writeUInt16LE(name.length, 26);
    locals.push(local, name, data);
    const cd = Buffer.alloc(46);
    cd.writeUInt32LE(0x02014b50, 0);
    cd.writeUInt16LE((3 << 8) | 20, 4); // made by Unix, so the modes come through where they matter
    cd.writeUInt16LE(20, 6);
    cd.writeUInt16LE(0x0800, 8);
    cd.writeUInt16LE(method, 10);
    cd.writeUInt16LE(time, 12);
    cd.writeUInt16LE(date, 14);
    cd.writeUInt32LE(crc, 16);
    cd.writeUInt32LE(data.length, 20);
    cd.writeUInt32LE(raw.length, 24);
    cd.writeUInt16LE(name.length, 28);
    cd.writeUInt32LE((((e.dir ? 0o040000 : 0o100000) | e.mode) << 16 | (e.dir ? 0x10 : 0)) >>> 0, 38);
    cd.writeUInt32LE(offset, 42);
    central.push(cd, name);
    offset += local.length + name.length + data.length;
  }
  const cdBuf = Buffer.concat(central);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(entries.length, 8);
  end.writeUInt16LE(entries.length, 10);
  end.writeUInt32LE(cdBuf.length, 12);
  end.writeUInt32LE(offset, 16);
  if (offset > 0xffffffff || entries.length > 0xffff) throw new Error('too large for a zip without zip64');
  return Buffer.concat([...locals, cdBuf, end]);
}

// ---------------------------------------------------------------- checksums

export const ARCHIVE_RE = /^xenolith-[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.]+)?-(?:darwin|linux|win32)-(?:arm64|x64)\.(?:tar\.gz|zip)$/;

/** Writes `dir`/SHA256SUMS over the release archives in `dir`, in the form `shasum -a 256 -c` reads. */
export function writeSums(dir) {
  const names = fs.readdirSync(dir).filter((n) => ARCHIVE_RE.test(n)).sort();
  if (!names.length) throw new Error(`no release archives in ${dir}`);
  const lines = names.map((n) => `${crypto.createHash('sha256').update(fs.readFileSync(path.join(dir, n))).digest('hex')}  ${n}\n`);
  fs.writeFileSync(path.join(dir, 'SHA256SUMS'), lines.join(''));
  return names;
}
