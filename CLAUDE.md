# shape.ai

Visual group canvas for humans and AI agents — an infinite, local-first canvas
that agents can review and extend through MCP.

## Architecture

shape.ai is an infinite canvas built in Rust on `wgpu`. The canvas is the
product, so everything performance-sensitive — drawing, navigation, hit testing,
layout, LOD — lives in the Rust cores.

The one rule: **keep canvas logic in the Rust cores, keep every shell thin.**
`crates/` holds the cores: `scene-core` (model + the one op-apply),
`client-runtime` (collaboration semantics no shell reimplements),
`renderer-core` (pure CPU) + `renderer-wgpu` (GPU) render seam, `storage-core`,
`coordination` (the scale-out lease/presence/pubsub `Coordinator` seam),
`server`, and `platform-contract` — what a shell may see, so a new shell needs
no core change. `platforms/` holds one shell per OS: `web/`
is live, `macos/`/`ios/`/`android/` reserved empty dirs. A shell only forwards OS
input, gives a surface, implements host ports, draws product UI; it owns no
canonical state, holding at most a render-only projection the core hands it —
never authoring a decision or the next op from a shell-side copy. Directory-scoped
rules live in each dir's `CLAUDE.md`.

The cores stay portable because native targets (macOS Metal, iOS) are intended —
hence no time, randomness, threads, or I/O in them. The server stays as
stateless as possible for scale-out.

## Conventions

Performance is the top-priority target in every change: never ship a known-slow
"just make it work first" implementation — redesign until it can hit the bar
(transform-only updates, zero per-frame re-tessellation, no avoidable allocations
on the hot path).

Comments carry only what the code can't say — a non-obvious constraint, a why, a
hazard. Cut narration that restates the next line, task/changelog markers, and edit
history (that lives in the commit); a comment that only repeats the code is deleted.

Build no abstraction past what a present caller needs — no indirection, option, or
generality for one use site or a hypothetical future. The simplest code that clears
the performance bar beats a flexible one that doesn't; collapse a layer the moment
it has a single caller.

The pure cores stay pointer-width-agnostic: no 32-bit address assumptions, so a
future 64-bit wasm (Memory64) port is a target-triple flip, not a rewrite —
workspace lints (`Cargo.toml`) deny width-narrowing casts.

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

The web shell owns its build toolchain (`platforms/web/package.json`, run from
that dir); Rust goes through `scripts/renderer-toolchain.sh` (the pinned
toolchain is otherwise not on `PATH`); the root `Makefile` wraps every
cross-cutting combo and running the app. Prefer the wrapper to the command it
wraps — serve, build, and test through `make` (read it and
`platforms/web/package.json` for the exact targets), and reach for a raw
`cargo`/`npm` invocation only for a one-off no target covers, never as a
shortcut past a target that already exists.
