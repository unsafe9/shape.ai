# shape.ai

Visual group canvas for humans and AI agents — an infinite, local-first canvas
that agents can review and extend through MCP.

## Architecture

shape.ai is an infinite canvas built in Rust on `wgpu`. The canvas is the
product, so everything performance-sensitive — drawing, navigation, hit testing,
layout, LOD — lives in the Rust core.

The one rule: **keep canvas logic in the Rust core and keep every platform layer
thin.** The boundary runs core → platform adapter → app shell: a platform layer
carries only what the OS forces it to (product UI, plus OS integration like
input, IME, clipboard, and the GPU surface); everything else belongs in the core.
This is the design intent, not something the code fully enforces yet — pushing
canvas behavior up into the shell, or leaking platform assumptions down into the
core, both erode the portability the boundary exists to protect.

Both server and client should be pure Rust, with the non-Rust shell (today
Svelte) kept as thin as possible. The core stays portable because native targets
(macOS Metal, iOS) are intended — which is why it holds no time, randomness,
threads, or I/O. The server should stay as stateless as possible, for high
parallelism and future scale-out.

## Conventions

Performance is the top-priority target in every change: never ship a known-slow
"just make it work first" implementation — redesign until it can hit the bar
(transform-only updates, zero per-frame re-tessellation, no avoidable allocations
on the hot path).

The pure cores stay pointer-width-agnostic: no 32-bit address assumptions, so a
future 64-bit wasm (Memory64) port is a target-triple flip, not a rewrite.

Shortcuts and gestures have a single source: a click/shortcut command lives in
scene-core `commands.rs` (`object_command_catalog`); a hold-key gesture lives in
scene-core `gestures.rs` (`object_gesture_catalog`). Both export JSON over
`wasm_api`, and the settings modal renders them read-only — register a feature in
its catalog once and it self-documents, with no second place to update. So any
new key/modifier/button-hold or click behavior starts in the catalog, and the
shell routes off its mirrored binding predicate; never branch behavior on a raw
`event.shiftKey`/key-literal read in shell code, which skips the catalog and
never reaches the modal. Verify by confirming the entry appears in the settings
(Cmd+,) list with no modal edit.

Every behavior change ships a falsifiable assertion — a test that fails when the
behavior is wrong, not merely one that compiles.

## Operating Notes

Op-apply has exactly one implementation — the Rust core, which both the server
and the client (via wasm) run. Never add a second op-apply in any language,
including in tests: tests must drive the real core.

## Commands

Run cargo/wasm-pack through `scripts/renderer-toolchain.sh` — the pinned toolchain
is otherwise not on `PATH`.

- `npm run build` — build the client (wasm + vite)
- `cargo run -p shape_server` — serve the app on :8787
- `npm run dev` — iterative client against a running server
- `cargo test --workspace` · `npm run test:unit` — test gates
