# Rust runtime

The shipping programs are xenolith, xenolith-service and xenolith-tray. The core crate owns storage, search, the curator, held changes, sync and Forget, and takes XenoNet's libraries (xenonet-id, -log, -authz) as path dependencies. The mcp crate owns the MCP server's tools, resources and prompts. The CLI (xenolith) owns the stdio server, the hook, the commands and installation, on the core alone, so the hook's program stays small. The service crate (xenolith-service) owns the HTTP service, the curator and the Windows service, and the net crate (xenolith-net, on xenonet-net) sync between machines; only the service takes it. The tray owns native window and platform integration. scripts/xenolith-lock.mjs, in the repository's root, checks that Xenolith's lockfile agrees with the root's on everything the libraries reach.

## Status

The Rust core, server, curator, Ask pipeline, service, tray and installer are the only implementation. The earlier Node runtime and the C# service and tray are retired and removed from this source tree.

## Build and verify

Build the window with npm run build. Build the Rust workspace with cargo build --release --locked --workspace from rust/, on the toolchain rust/rust-toolchain.toml pins. Before pushing Rust changes, run scripts/check-rust.sh in Linux, WSL or macOS. It checks formatting, lint and unit tests, including cross-target lint where supported.

npm test runs the JavaScript behavior suites against the Rust programs. See AGENTS.md for the full commands and the isolated verification skill. The Xenolith workflow (.github/workflows/xenolith.yml) runs the same checks on Windows, Linux and macOS.

Use scripts/bench.mjs to measure the program on synthetic data. Keep raw machine-specific output outside tracked files. Report hardware, sample counts and measurement conditions only when those details are intended for publication.
