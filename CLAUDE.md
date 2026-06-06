# shape.ai

Visual group canvas for humans and AI agents — an infinite, local-first canvas
that agents can review and extend through MCP.

## Technical Background

shape.ai is an infinite canvas and design system built in Rust on top of `wgpu`.
The canvas is the product, so everything performance-sensitive about it — drawing,
navigation, hit testing, layout, LOD — lives in the Rust core.

The web (WASM + WebGPU) is the only target shipping today, but multi-platform
reach — macOS via Metal, and mobile/iOS — is a deliberate design constraint, not a
someday-nice-to-have. `wgpu` already abstracts those native backends, so the core
is meant to compile for them without forking the scene model.

The architecture turns on one rule: **keep canvas logic in the Rust core and keep
every platform layer thin.** A platform layer — the web today, a native app
later — carries only what the OS forces it to: essential product UI plus OS
integration such as input, IME, clipboard, and the GPU surface. Everything else
belongs in the core. This guards against two equal-and-opposite failure modes:
pushing canvas behavior up into UI/component state, and leaking platform
assumptions down into the core. Either one erodes the portability the boundary
exists to protect.

That boundary runs core → platform adapter → app shell. The adapter is a thin
per-target seam; the app shell owns product UI and orchestration and talks to the
canvas only through a narrow imperative handle plus an event stream, never by
reaching into its internals. The web shell is **Svelte** (the project has fully
cut over from React); a future macOS shell would be SwiftUI, changing only the
language and the adapter.

