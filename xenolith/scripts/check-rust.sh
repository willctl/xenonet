#!/usr/bin/env bash
# Formats, lints and tests the Rust workspace for every OS CI builds, from one Linux, WSL or macOS
# shell, so code behind #[cfg(windows)] or #[cfg(target_os = "macos")] is checked before a push.
# CI (.github/workflows/xenolith.yml) runs the same lints on each OS, but only after the push lands.
#
#   scripts/check-rust.sh
#
# The toolchain is the one rust/rust-toolchain.toml pins. On Linux the tray needs GTK, WebKitGTK and
# the app indicator to build (libgtk-3-dev libwebkit2gtk-4.1-dev libayatana-appindicator3-dev).
#
# Needs rustup. It adds the x86_64-pc-windows-gnu and aarch64-apple-darwin targets if missing. Off
# macOS, the tray is linted for macOS only on a Mac: its macOS dependencies need Apple's SDK.
set -euo pipefail
cd "$(dirname "$0")/../rust"
host=$(rustc -vV | sed -n 's/^host: //p')
step() { printf '\n== %s\n' "$*"; }

step "rustfmt"
cargo fmt --all --check
step "clippy ($host)"
cargo clippy --workspace --all-targets --locked -- -D warnings
if [[ "$host" != *windows* ]]; then
  rustup target add x86_64-pc-windows-gnu >/dev/null 2>&1 || true
  step "clippy (x86_64-pc-windows-gnu)"
  cargo clippy --workspace --target x86_64-pc-windows-gnu --all-targets --locked -- -D warnings
fi
if [[ "$host" != *apple-darwin ]]; then
  rustup target add aarch64-apple-darwin >/dev/null 2>&1 || true
  step "clippy (aarch64-apple-darwin: core and cli; the tray needs a Mac)"
  cargo clippy --target aarch64-apple-darwin -p xenolith-core -p xenolith-cli --all-targets --locked -- -D warnings
fi
step "tests ($host)"
cargo test --workspace --locked
printf '\nAll Rust checks passed for %s, Windows and macOS.\n' "$host"
