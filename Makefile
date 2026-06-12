.PHONY: wasm types build dev serve web test test-rust test-web test-renderer check-wasm

wasm:
	cd platforms/web && npm run scene:wasm:build && npm run renderer:wasm:build

types:
	cd platforms/web && npm run types:gen

build:
	cd platforms/web && npm run build

serve:
	scripts/renderer-toolchain.sh cargo run -p shape_server

web:
	cd platforms/web && npm run dev

dev:
	@echo "Run the server and the web client in two shells:"
	@echo "  make serve"
	@echo "  make web"

test: test-rust test-web

test-rust:
	scripts/renderer-toolchain.sh cargo test --workspace

test-web:
	cd platforms/web && npm run test:unit

test-renderer:
	scripts/renderer-toolchain.sh cargo test --manifest-path crates/renderer-wgpu/Cargo.toml --features wgpu-probe

check-wasm:
	scripts/renderer-toolchain.sh cargo check -p shape_scene_core --target wasm32-unknown-unknown
	scripts/renderer-toolchain.sh cargo check -p shape_storage_core --target wasm32-unknown-unknown
	scripts/renderer-toolchain.sh cargo check -p shape_client_runtime --target wasm32-unknown-unknown
