#!/usr/bin/env sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
toolchain_dir="$repo_root/.renderer-toolchains"
cargo_bin="$toolchain_dir/cargo/bin"

if [ -x "$cargo_bin/cargo" ]; then
  export RUSTUP_HOME="$toolchain_dir/rustup"
  export CARGO_HOME="$toolchain_dir/cargo"
  export PATH="$cargo_bin:$PATH"
fi

exec "$@"
