#!/bin/sh
# Runs one fuzz target (see "Fuzzing" in docs/guide/development.md):
#
#   fuzz/run.sh <target> [seconds] [libFuzzer options…]
#
# New inputs go to fuzz/corpus/<target>, which git ignores; the committed
# seeds in fuzz/seeds/<target> are read too. A crash is saved to
# fuzz/artifacts/<target>. Without seconds, or with 0, it runs until
# stopped.
set -eu
cd "$(dirname "$0")/.."
target=$1
shift
seconds=${1:-0}
[ $# -gt 0 ] && shift
# libFuzzer needs a nightly Rust, which the checkout's rust-toolchain.toml
# would otherwise override: its programs go first on the PATH, as
# scripts/cargo.mjs does for the pinned one.
if bin=$(rustup which cargo --toolchain nightly 2>/dev/null); then
  bin=$(dirname "$bin")
  PATH="$bin:$PATH" RUSTC="$bin/rustc"
  export PATH RUSTC
fi
mkdir -p "fuzz/corpus/$target"
seeds=
[ -d "fuzz/seeds/$target" ] && seeds="fuzz/seeds/$target"
exec cargo fuzz run --fuzz-dir fuzz "$target" "fuzz/corpus/$target" $seeds -- \
  -max_total_time="$seconds" -timeout=10 -print_final_stats=1 -dict=fuzz/dict/json.dict "$@"
