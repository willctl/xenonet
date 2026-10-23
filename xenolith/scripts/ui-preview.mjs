// npm run ui:preview [-- --port 47859] [-- --wiki <dir>] [-- --no-ask]
//
// Serves the window from dist/runtime/ui (run `npm run build` after changing ui/) on its own port,
// over a snapshot copy of your shared context folder in .tmp/ui-preview, so trying the window
// never touches the real one. Open http://127.0.0.1:<port>/ui/ in a browser. The service keeps
// running as is. Ask works too: a `xenolith curator --asks-only` answers questions on the snapshot
// with the curator's own Codex sign-in (it files no notes); --no-ask leaves it out. The programs
// are the ones in rust/target/release (or XENOLITH_RUST_BIN).

import { spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { P, readJson } from './lib.mjs';
import { rustBin } from '../test/impl.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));
const args = process.argv.slice(2);
const opt = (n, d) => (args.includes(n) ? args[args.indexOf(n) + 1] : d);
const port = Number(opt('--port', 47859));
const config = (await readJson(P.config).catch(() => null)) || {};
const source = path.resolve(opt('--wiki', config.wikiDir || P.defaultWiki));
const dir = path.join(root, '.tmp', 'ui-preview');
const wikiDir = path.join(dir, 'wiki');
const home = path.join(dir, 'home');

fs.rmSync(dir, { recursive: true, force: true });
fs.cpSync(source, wikiDir, { recursive: true, filter: (src) => !src.split(path.sep).includes('.locks') });
fs.rmSync(path.join(wikiDir, '.curator', 'asks'), { recursive: true, force: true });
fs.mkdirSync(home, { recursive: true });
fs.writeFileSync(path.join(home, 'config.json'), JSON.stringify({ writeMode: 'curated', curator: config.curator, ask: config.ask }));
console.log(`snapshot of ${source} -> ${wikiDir}`);

const env = { ...process.env, XENOLITH_DIR: wikiDir, XENOLITH_HOME: home, XENOLITH_UI_DIR: path.join(root, 'dist', 'runtime', 'ui') };
const children = [spawn(rustBin(), ['serve', '--http', '--port', String(port)], { env, stdio: ['ignore', 'inherit', 'inherit'] })];
if (!args.includes('--no-ask')) children.push(spawn(rustBin(), ['curator', '--asks-only'], { env, stdio: ['ignore', 'inherit', 'inherit'] }));
console.log(`window preview: http://127.0.0.1:${port}/ui/`);
children[0].on('exit', (code) => {
  children.forEach((c) => c.kill());
  process.exit(code ?? 0);
});
for (const sig of ['SIGINT', 'SIGTERM']) process.on(sig, () => children.forEach((c) => c.kill()));
