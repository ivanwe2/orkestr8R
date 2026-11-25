#!/usr/bin/env bash
# Builds a native Windows `ork.exe` in a rust container (MinGW, the x86_64-pc-windows-gnu target),
# for Windows machines that can't link Rust themselves (no MSVC build tools). Same toolchain as
# rust-toolchain.toml. The result goes to target-win/ork.exe.
#
#   scripts/build-windows.sh
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && (pwd -W 2>/dev/null || pwd))"
toolchain="$(sed -n 's/^channel = "\(.*\)"/\1/p' "$root/rust-toolchain.toml")"

export MSYS_NO_PATHCONV=1   # Git Bash: don't rewrite /work into a Windows path
docker run --rm -i \
  -v "$root:/work" -w /work \
  -v orkestr8r-target:/target -e CARGO_TARGET_DIR=/target \
  -v orkestr8r-cargo-registry:/usr/local/cargo/registry \
  -v orkestr8r-rustup:/usr/local/rustup \
  "rust:$toolchain" bash -c '
    set -e
    apt-get update -qq >/dev/null
    apt-get install -y -qq gcc-mingw-w64-x86-64 >/dev/null
    rustup target add x86_64-pc-windows-gnu >/dev/null
    cargo build --release --locked --target x86_64-pc-windows-gnu
    mkdir -p /work/target-win
    cp /target/x86_64-pc-windows-gnu/release/ork.exe /work/target-win/ork.exe
  '
echo "built target-win/ork.exe"
