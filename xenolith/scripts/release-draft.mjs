// The release's checks and its GitHub draft (.github/workflows/xenolith-release.yml).
//
//   node scripts/release-draft.mjs --check [--tag xenolith-v<version>]
//     package.json, package-lock.json and rust/Cargo.toml name one version, the tag (when given, or
//     GITHUB_REF_NAME on a tag build) is xenolith-v<that version>, and docs/releases/<version>.md exists.
//   node scripts/release-draft.mjs --assets <dir>
//     The checks, then the archives in <dir>: one for every target, and SHA256SUMS right for each.
//   node scripts/release-draft.mjs [--dir dist/release]
//     Both, then a draft release "Xenolith <version>" for the tag with the archives and SHA256SUMS
//     attached and the notes from docs/releases/. A draft that exists gets its files replaced; a
//     published release is never touched. The draft is not marked latest: XenoNet's updates read
//     this repository's latest release.

import crypto from 'node:crypto';
import { execFileSync, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { ARCHIVE_RE } from './archive.mjs';
import { TARGETS } from './package.mjs';

const REPO = path.resolve(fileURLToPath(new URL('..', import.meta.url)));

export function checkRelease({ tag = null, root = REPO } = {}) {
  const read = (f) => fs.readFileSync(path.join(root, f), 'utf8');
  const pkg = JSON.parse(read('package.json'));
  const lock = JSON.parse(read('package-lock.json'));
  const cargo = read('rust/Cargo.toml').match(/^\[workspace\.package\][^[]*?^version = "([^"]+)"/m)?.[1];
  const v = pkg.version;
  const problems = [];
  if (lock.version !== v || lock.packages?.['']?.version !== v) problems.push(`package-lock.json says ${lock.version}: run npm install to bring it to ${v}`);
  if (cargo !== v) problems.push(`rust/Cargo.toml [workspace.package] says ${cargo}, package.json ${v}`);
  if (tag && tag !== `xenolith-v${v}`) problems.push(`the tag ${tag} is not xenolith-v${v}`);
  const notes = path.join('docs', 'releases', `${v}.md`);
  if (!fs.existsSync(path.join(root, notes)) || !read(notes).trim()) problems.push(`${notes} is missing: write the release notes`);
  if (problems.length) throw new Error(problems.join('\n'));
  return { version: v, notes: path.join(root, notes) };
}

/** The archives and SHA256SUMS in `dir`, each target's present and every sum right. Returns the paths to attach. */
export function checkAssets(dir, version) {
  const want = Object.keys(TARGETS).map((t) => `xenolith-${version}-${t}.${t.startsWith('win32') ? 'zip' : 'tar.gz'}`);
  const have = fs.readdirSync(dir).filter((n) => ARCHIVE_RE.test(n));
  const missing = want.filter((n) => !have.includes(n));
  const extra = have.filter((n) => !want.includes(n));
  if (missing.length || extra.length) throw new Error(`${dir}: ${missing.length ? `missing ${missing.join(', ')}` : ''}${missing.length && extra.length ? '; ' : ''}${extra.length ? `not this release's: ${extra.join(', ')}` : ''}`);
  const sums = fs.readFileSync(path.join(dir, 'SHA256SUMS'), 'utf8').trim().split('\n').map((l) => l.split(/\s+\*?/));
  for (const n of want) {
    const line = sums.find(([, f]) => f === n);
    const actual = crypto.createHash('sha256').update(fs.readFileSync(path.join(dir, n))).digest('hex');
    if (!line || line[0] !== actual) throw new Error(`SHA256SUMS has ${line ? 'a wrong sum' : 'no line'} for ${n}`);
  }
  if (sums.length !== want.length) throw new Error(`SHA256SUMS lists ${sums.length} files, not ${want.length}`);
  return [...want, 'SHA256SUMS'].map((n) => path.join(dir, n));
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  const opt = (n) => {
    const i = args.indexOf(n);
    return i >= 0 ? args[i + 1] : undefined;
  };
  const tag = opt('--tag') || (process.env.GITHUB_REF_TYPE === 'tag' ? process.env.GITHUB_REF_NAME : null);
  try {
    const { version, notes } = checkRelease({ tag });
    console.log(`Xenolith ${version}: versions agree${tag ? `, tag ${tag}` : ''}, notes ${path.relative(REPO, notes)}`);
    if (opt('--assets')) {
      const assets = checkAssets(path.resolve(opt('--assets')), version);
      console.log(`assets: ${assets.map((a) => path.basename(a)).join(', ')}`);
    } else if (!args.includes('--check')) {
      if (!tag) throw new Error('a draft needs the tag: run it on a tag build or pass --tag xenolith-v<version>');
      const assets = checkAssets(path.resolve(opt('--dir') || path.join(REPO, 'dist', 'release')), version);
      const existing = spawnSync('gh', ['release', 'view', tag, '--json', 'isDraft'], { encoding: 'utf8' });
      if (existing.status === 0) {
        if (!JSON.parse(existing.stdout).isDraft) throw new Error(`${tag} is already published. Ship a new version instead of replacing its files.`);
        execFileSync('gh', ['release', 'upload', tag, ...assets, '--clobber'], { stdio: 'inherit' });
        execFileSync('gh', ['release', 'edit', tag, '--title', `Xenolith ${version}`, '--notes-file', notes], { stdio: 'inherit' });
      } else {
        execFileSync('gh', ['release', 'create', tag, ...assets, '--verify-tag', '--draft', '--latest=false', '--title', `Xenolith ${version}`, '--notes-file', notes], { stdio: 'inherit' });
      }
      console.log(`draft release ${tag}: ${assets.map((a) => path.basename(a)).join(', ')}`);
    }
  } catch (e) {
    console.error(`release: ${e.message}`);
    process.exit(1);
  }
}
