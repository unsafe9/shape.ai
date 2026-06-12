#!/usr/bin/env sh
# Generate the TS wire types from the scene-core serde surface via ts-rs.
#
#   scripts/types-gen.sh            # regenerate the committed types in place
#   scripts/types-gen.sh --check    # regenerate to a temp dir and diff (CI gate)
#
# The Rust side is the single source of truth: the `ts-gen` feature derives
# `ts_rs::TS` on the wire types and an export test writes them out. We point
# ts-rs's TS_RS_EXPORT_DIR at the committed `generated/` dir (or a temp clone in
# --check mode) and run that test through the pinned toolchain wrapper.
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
out_dir="$repo_root/platforms/web/shared/generated"
wrapper="$repo_root/scripts/renderer-toolchain.sh"

run_gen() {
  # ts-rs honours TS_RS_EXPORT_DIR for `#[ts(export_to=...)]`; the const test reads it too.
  # The `export_` filter matches both the ts-rs `export_bindings_*` unit tests
  # (the wire types) and our `export_geometry_const` integration test; nothing else
  # in the crate is named `export_*`.
  TS_RS_EXPORT_DIR="$1" "$wrapper" cargo test \
    -p shape_scene_core --features ts-gen \
    export_
}

if [ "${1:-}" = "--check" ]; then
  tmp_dir=$(mktemp -d)
  trap 'rm -rf "$tmp_dir"' EXIT
  run_gen "$tmp_dir" >/dev/null 2>&1 || { echo "types:check — generation failed" >&2; run_gen "$tmp_dir"; exit 1; }
  if ! diff -ru "$out_dir" "$tmp_dir"; then
    echo "" >&2
    echo "types:check FAILED — generated TS drifted from the committed surface." >&2
    echo "Run 'npm run types:gen' and commit platforms/web/shared/generated/." >&2
    exit 1
  fi
  echo "types:check OK — generated TS matches the committed surface."
else
  run_gen "$out_dir"
  echo "types:gen wrote $out_dir"
fi
