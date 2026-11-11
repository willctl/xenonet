// The programs every conversation starts stay light: `xenolith` (the hook and the stdio server) and
// the tray, and the crates under them, reach none of the async runtime, HTTP client, TLS or
// transport crates that XenoNet's network brings. Those are xenolith-net's and xenolith-service's
// alone. Fails naming each one found, on any target.
//
//   node scripts/light.mjs
//
// scripts/check-rust.sh and .github/workflows/xenolith.yml run it.
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const rust = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'rust');
const LIGHT = ['xenolith-core', 'xenolith-mcp', 'xenolith-cli', 'xenolith-tray'];
const HEAVY = ['tokio', 'reqwest', 'rustls', 'hyper', 'iroh', 'quinn'];

let found = 0;
for (const p of LIGHT) {
  const r = spawnSync('cargo', ['tree', '-p', p, '-e', 'normal', '--target', 'all', '--locked', '--prefix', 'none', '-f', '{p}'], { cwd: rust, encoding: 'utf8', maxBuffer: 1 << 26, windowsHide: true });
  if (r.status !== 0) {
    process.stderr.write(`cargo tree -p ${p} failed:\n${r.stderr}`);
    process.exit(1);
  }
  const heavy = [...new Set(r.stdout.split('\n').map((l) => l.split(' ')[0]).filter((n) => HEAVY.includes(n)))];
  if (heavy.length) {
    found++;
    process.stderr.write(`${p} reaches ${heavy.join(', ')}: only xenolith-net and xenolith-service may (cargo tree -p ${p} -e normal -i ${heavy[0]} shows how)\n`);
  }
}
if (found) process.exit(1);
process.stderr.write(`${LIGHT.join(', ')} reach none of ${HEAVY.join(', ')}.\n`);
