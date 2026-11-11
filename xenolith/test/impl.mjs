// The programs the suites drive: the Rust `xenolith` (XENOLITH_RUST_BIN, else
// rust/target/{release,debug}/xenolith) and `xenolith-service` beside it (XENOLITH_SERVICE_BIN to
// name another). The Node runtime is retired; XENOLITH_IMPL may still say "rust", and anything else
// is a mistake worth stopping on.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const REPO = fileURLToPath(new URL('..', import.meta.url));
if (process.env.XENOLITH_IMPL && process.env.XENOLITH_IMPL !== 'rust') {
  throw new Error(`XENOLITH_IMPL=${process.env.XENOLITH_IMPL}: the Node runtime is retired; the suites run the Rust programs only`);
}

const EXE = process.platform === 'win32' ? '.exe' : '';

/** `xenolith`: the stdio server, the hook and the commands. */
export function rustBin() {
  if (process.env.XENOLITH_RUST_BIN) return process.env.XENOLITH_RUST_BIN;
  for (const p of ['release', 'debug']) {
    const f = path.join(REPO, 'rust', 'target', p, `xenolith${EXE}`);
    if (fs.existsSync(f)) return f;
  }
  throw new Error('Build the Rust programs first (cargo build --release in rust/), or set XENOLITH_RUST_BIN');
}

/** `xenolith-service`: the HTTP service, the curator and the Windows service, beside xenolith. */
export function serviceBin() {
  if (process.env.XENOLITH_SERVICE_BIN) return process.env.XENOLITH_SERVICE_BIN;
  const f = path.join(path.dirname(rustBin()), `xenolith-service${EXE}`);
  if (!fs.existsSync(f)) throw new Error(`xenolith-service isn't beside ${rustBin()}: build the workspace (cargo build --release in rust/), or set XENOLITH_SERVICE_BIN`);
  return f;
}

// The server finds the window's files next to itself; in tests they are in dist/runtime/ui.
process.env.XENOLITH_UI_DIR ||= path.join(REPO, 'dist', 'runtime', 'ui');

/** [command, args] that start the server with these flags: the HTTP service with --http, else stdio. */
export function srv(...args) {
  return [args.includes('--http') ? serviceBin() : rustBin(), ['serve', ...args]];
}

/** {command, args} for StdioClientTransport. */
export function srvT(...args) {
  const [command, a] = srv(...args);
  return { command, args: a };
}

/** [command, args] that run the curator with these flags. */
export function curatorCmd(...args) {
  return [serviceBin(), ['curator', ...args]];
}

/** [command, args] that run the SessionStart hook. */
export function hookCmd() {
  return [rustBin(), ['hook']];
}
