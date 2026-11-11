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
# Needs rustup. It adds the x86_64-pc-windows-gnu target if missing, and the Windows lint needs
# MinGW-w64's gcc (x86_64-w64-mingw32-gcc): XenoNet's libraries build C (ring, SQLite). For the same
# reason macOS is linted only on a Mac, where Apple's SDK is; elsewhere CI's macOS job lints it.
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
step "the hook's program and the tray stay light"
node ../scripts/light.mjs
step "tests ($host)"
cargo test --workspace --locked
if [[ "$host" == *apple-darwin ]]; then
  printf '\nAll Rust checks passed for %s and Windows.\n' "$host"
else
  printf '\nAll Rust checks passed for %s and Windows; CI lints macOS.\n' "$host"
fi
