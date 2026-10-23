# Rust runtime

The shipping programs are xenolith and xenolith-tray. The core crate owns storage, search, the curator, held changes, sync and Forget. The CLI owns transports, installation and the service. The tray owns native window and platform integration.

## Status

The Rust core, server, curator, Ask pipeline, service, tray and installer are the only implementation. The earlier Node runtime and the C# service and tray are retired and removed from this source tree.

## Build and verify

Build the window with npm run build. Build the Rust workspace with cargo build --release --locked --workspace from rust/, on the toolchain rust/rust-toolchain.toml pins. Before pushing Rust changes, run scripts/check-rust.sh in Linux, WSL or macOS. It checks formatting, lint and unit tests, including cross-target lint where supported.

npm test runs the JavaScript behavior suites against the Rust programs. See AGENTS.md for the full commands and the isolated verification skill. The Xenolith workflow (.github/workflows/xenolith.yml) runs the same checks on Windows, Linux and macOS.

Use scripts/bench.mjs to measure the program on synthetic data. Keep raw machine-specific output outside tracked files. Report hardware, sample counts and measurement conditions only when those details are intended for publication.
