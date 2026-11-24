#!/usr/bin/env bash
# Runs cargo in a rust container, for machines that can't link Rust natively
# (e.g. Windows without the MSVC build tools). Same toolchain as rust-toolchain.toml.
#
#   scripts/dcargo.sh test --all-features
#   scripts/dcargo.sh clippy --all-targets --all-features -- -D warnings
#
# target/, the cargo registry and rustup live in named Docker volumes: faster than the bind mount,
# kept apart from native builds, and nothing is downloaded twice.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && (pwd -W 2>/dev/null || pwd))"
toolchain="$(sed -n 's/^channel = "\(.*\)"/\1/p' "$root/rust-toolchain.toml")"

export MSYS_NO_PATHCONV=1   # Git Bash: don't rewrite /work into a Windows path
exec docker run --rm -i \
  -v "$root:/work" -w /work \
  -v orkestr8r-target:/target -e CARGO_TARGET_DIR=/target \
  -v orkestr8r-cargo-registry:/usr/local/cargo/registry \
  -v orkestr8r-rustup:/usr/local/rustup \
  -e RUSTFLAGS="${RUSTFLAGS:-}" -e CARGO_TERM_COLOR="${CARGO_TERM_COLOR:-never}" \
  "rust:$toolchain" cargo "$@"
