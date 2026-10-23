// Public-source checks. Private blocked terms may be supplied in .git/privacy-blocked.json.
import fs from 'node:fs';
import { execFileSync } from 'node:child_process';

const git = (...args) => execFileSync('git', args, { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 }).trimEnd();
const staged = process.argv.includes('--staged');
const history = process.argv.includes('--history');
const failures = [];
// The public identity: the address will@willctl.com, under the name Will (as the project's commits
// are made) or willctl.
const PUBLIC_EMAIL = 'will@willctl.com';
const PUBLIC_NAMES = ['Will', 'willctl'];
const isPublic = (ident) => {
  const m = ident.match(/^(.*) <([^>]*)>/);
  return Boolean(m) && m[2] === PUBLIC_EMAIL && PUBLIC_NAMES.includes(m[1]);
};
if (!process.argv.includes('--files-only')) {
  for (const kind of ['AUTHOR', 'COMMITTER']) {
    if (!isPublic(git('var', `GIT_${kind}_IDENT`))) failures.push(`${kind.toLowerCase()} identity: use the repository's public identity (Will <${PUBLIC_EMAIL}>)`);
  }
}
if (history) {
  for (const row of git('log', '--all', '--format=%an <%ae>%x09%cn <%ce>').split('\n')) {
    if (row.split('\t').some(x => !isPublic(x))) failures.push('history contains a non-public commit identity');
  }
}
let blocked = [];
const privateRules = git('rev-parse', '--git-path', 'privacy-blocked.json');
if (fs.existsSync(privateRules)) blocked = JSON.parse(fs.readFileSync(privateRules, 'utf8'));
const files = git('ls-files', '-z').split('\0').filter(Boolean);
for (const file of files) {
  if (!staged && !fs.existsSync(file)) continue;
  if (/(^|\/)\.env(?:\.|$)|\.(pem|pfx|p12|key|dump|sqlite|db|bundle)$/i.test(file)) failures.push(`${file}: private file type`);
  let text;
  if (staged) { try { text = git('show', `:${file}`); } catch { continue; } }
  else text = fs.readFileSync(file, 'utf8');
  if (blocked.some(term => (file + '\n' + text).toLowerCase().includes(term.toLowerCase()))) failures.push(`${file}: contains a privately blocked term`);
  const normalized = text.replace(/\\+/g, '/');
  for (const match of normalized.matchAll(/(?:[a-z]:)?\/(?:Users|home)\/([^/\s"'`<>]+)/gi)) {
    if (!['example', 'x', 'w', 'A', 'test', 'user', 'runner', 'USERNAME', '$USER', '${USER}'].includes(match[1])) failures.push(`${file}: contains a personal home directory`);
  }
}
for (const file of ['package.json','plugin/.claude-plugin/plugin.json','plugin/.codex-plugin/plugin.json']) {
  const obj = JSON.parse(staged ? git('show', `:${file}`) : fs.readFileSync(file, 'utf8'));
  if ((typeof obj.author === 'string' ? obj.author : obj.author?.name) !== 'willctl') failures.push(`${file}: author must be the public identity`);
}
if (failures.length) {
  console.error([...new Set(failures)].join('\n'));
  process.exit(1);
}
console.log(`Public-source checks passed for ${files.length} tracked files${history ? ' and all local refs' : ''}.`);
