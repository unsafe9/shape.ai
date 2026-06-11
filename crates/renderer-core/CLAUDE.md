# renderer-core — the pure CPU renderer

This is the device-independent half of the renderer: tessellation, stroke
expansion, curve LOD, outline/region derivation, text layout, hit testing, the
render model, object geometry build, the blur kernel.

If a `wgpu` or `web_sys` type appears in this crate, the boundary is broken. The
GPU pipelines, surface lifecycle, and the `#[wasm_bindgen]` surface live in
`renderer-wgpu`, which depends on this crate by path — never the reverse.

Every render decision must be assertable on a GPU-less host: a function decides,
host tests prove it, and `renderer-wgpu` only executes the result. If a decision
can only be checked by running a GPU, it is in the wrong crate.

The perf bar in renderer terms: a transform-only update (pan/zoom/move) must
never force a full geometry rebuild or re-tessellation. Keep transform and
geometry on separate paths so motion stays transform-only.
