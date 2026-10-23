// Builds the window's web app (ui/, React) into dist/runtime/ui/, which `xenolith serve --http`
// serves at /ui/ (the installer copies it next to the program), after checking that package.json
// and rust/Cargo.toml carry the same version.

import { build } from 'esbuild';
import fs from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const pkg = JSON.parse(await fs.readFile(path.join(root, 'package.json'), 'utf8'));
// The window shows package.json's version and the Rust programs report rust/Cargo.toml's: one release, one number.
const cargoVersion = (await fs.readFile(path.join(root, 'rust', 'Cargo.toml'), 'utf8')).match(/^\[workspace\.package\][^[]*?^version = "([^"]+)"/m)?.[1];
if (cargoVersion !== pkg.version) {
  console.error(`Version mismatch: package.json says ${pkg.version}, rust/Cargo.toml [workspace.package] says ${cargoVersion}. Set both to the same version.`);
  process.exit(1);
}
const outdir = path.join(root, 'dist', 'runtime');

await fs.rm(outdir, { recursive: true, force: true });
const uiOut = path.join(outdir, 'ui');
await build({
  absWorkingDir: root,
  entryPoints: { app: 'ui/src/main.jsx' },
  outdir: uiOut,
  bundle: true,
  platform: 'browser',
  format: 'iife',
  target: ['chrome120', 'edge120'],
  jsx: 'automatic',
  minify: true,
  charset: 'utf8',
  legalComments: 'none',
  logLevel: 'warning',
  define: { 'process.env.NODE_ENV': '"production"', __XENOLITH_VERSION__: JSON.stringify(pkg.version) },
});
await fs.copyFile(path.join(root, 'ui', 'index.html'), path.join(uiOut, 'index.html'));
await fs.copyFile(path.join(root, 'assets', 'icon', 'xenolith.svg'), path.join(uiOut, 'icon.svg'));

for (const f of await fs.readdir(uiOut, { withFileTypes: true })) {
  if (f.isDirectory()) continue;
  const { size } = await fs.stat(path.join(uiOut, f.name));
  console.log(`built ${path.relative(root, path.join(uiOut, f.name)).replace(/\\/g, '/')} (${(size / 1024).toFixed(0)} KB)`);
}
