#!/usr/bin/env bash
# Build the Rust and Zig contracts, then run the comparison.
# Requires: zig 0.15.x, stellar-cli 28.x, Rust with the wasm32v1-none target.
set -euo pipefail
cd "$(dirname "$0")/.."

for c in contracts/*/rust/Cargo.toml; do
    stellar contract build --manifest-path "$c" --out-dir out/rust
done
zig build
cargo run --release
