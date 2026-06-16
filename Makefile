.PHONY: wasm types build dev serve web test test-rust test-web test-renderer check-wasm lint

wasm:
	cd platforms/web && npm run scene:wasm:build && npm run renderer:wasm:build

types:
	cd platforms/web && npm run types:gen

build:
	cd platforms/web && npm run build

serve:
	scripts/renderer-toolchain.sh cargo run --release -p shape_server

web:
	cd platforms/web && npm run dev

dev:
	@echo "Run the server and the web client in two shells:"
	@echo "  make serve"
	@echo "  make web"

test: lint test-rust test-web

# The pointer-width-agnostic clippy gate. The workspace [lints] deny table
# (root Cargo.toml) makes width-narrowing casts (cast_possible_truncation/wrap,
# ptr_as_ptr/ref_as_ptr, trivial_casts) hard errors; this runs clippy over every
# member AND the workspace-excluded renderer-wgpu, which mirrors the table in its
# own Cargo.toml. `-D clippy::correctness` adds clippy's correctness group on top.
# A red gate fails `make test`.
#
# We deny the deny-TABLE lints, not a blanket `-D warnings`: `-D warnings` would
# turn clippy's whole warn-by-default STYLE backlog (derivable_impls,
# too_many_arguments, large_enum_variant on the op enum, ...) into errors across
# untouched core crates — a workspace-wide style refactor outside this gate's
# pointer-width purpose and the surgical-change rule. The deny table is the
# enforced contract; new width-narrowing casts still fail the gate.
#
# renderer-wgpu's GPU surface (`ShapeWebGpuRenderer`'s impls) is gated
# `#[cfg(target_arch = "wasm32")]`, so the HOST pass (line below) never compiles —
# nor lints — it: the device-boundary casts there would escape the deny table.
# The third pass lints renderer-wgpu FOR wasm32 (the real build target) so those
# `as f32`/`as u32` device casts hit the deny table too; an un-annotated narrowing
# cast added inside any wasm-only renderer method fails the gate. `--lib` because
# tests/benches don't link for wasm32-unknown-unknown without a runner.
lint:
	scripts/renderer-toolchain.sh cargo clippy --release --workspace --all-targets -- -D clippy::correctness
	scripts/renderer-toolchain.sh cargo clippy --release --manifest-path crates/renderer-wgpu/Cargo.toml --all-targets --features wgpu-probe -- -D clippy::correctness
	scripts/renderer-toolchain.sh cargo clippy --release --manifest-path crates/renderer-wgpu/Cargo.toml --lib --target wasm32-unknown-unknown -- -D clippy::correctness

test-rust:
	scripts/renderer-toolchain.sh cargo test --release --workspace

test-web:
	cd platforms/web && npm run test:unit

test-renderer:
	scripts/renderer-toolchain.sh cargo test --release --manifest-path crates/renderer-wgpu/Cargo.toml --features wgpu-probe

check-wasm:
	scripts/renderer-toolchain.sh cargo check --release -p shape_scene_core --target wasm32-unknown-unknown
	scripts/renderer-toolchain.sh cargo check --release -p shape_storage_core --target wasm32-unknown-unknown
	scripts/renderer-toolchain.sh cargo check --release -p shape_client_runtime --target wasm32-unknown-unknown
