# renderer-wgpu — the GPU half

The `wgpu` pipelines, WGSL shaders, surface lifecycle, and the
`#[wasm_bindgen]` renderer surface. The pure CPU half lives in `renderer-core`.

Build reality you must hold:

- Host tests run with the `wgpu-probe` feature on the HOST target, not wasm32.
  Drive logic from host-testable functions; reach a real device only behind the
  probe feature.
- WGSL compiles at RUNTIME, on a live device. A green wasm build does NOT catch a
  shader error — shaders are `include_str!`'d strings. So no decision belongs in
  WGSL or in a GPU-only path that a host test can't reach; lift it into
  `renderer-core`'s host-testable functions.
- This crate is workspace-EXCLUDED on purpose, to keep its pinned `wgpu`
  lockfile out of the workspace lock. It depends on `renderer-core` by path. Run
  its tests via the dedicated npm/cargo script, not `cargo test --workspace`.

`wgpu` is the single GPU HAL — it already covers WebGPU, Metal, Vulkan, and
DX12. Do NOT introduce a render-backend trait or a second backend abstraction; a
second real consumer must exist first.
