# client-runtime — collaboration session semantics

This crate owns the collaboration session logic shared by every shell: durable
outbox bookkeeping, peer presence, viewport-windowing decisions, and the sync
engine (optimistic local apply over scene-core, ownership-gated remote patches,
base-revision tracking, coalescing policy, reconnect reconcile). One Rust
implementation, run natively and on wasm32 — no shell reimplements it.

Pure: effects happen only through injected ports — the clock (`now_ms` /
`now: &str`), persistence (the outbox store), and transport (the engine's sink).
The crate computes decisions; shells drive the timers and perform the IO. A
coalescing window or a re-subscribe is decided here and *fired* by the shell.

New collaboration behavior starts HERE, in Rust. Never in a shell's adapter
layer (`platforms/web/runtime/*` and its peers are adapters wiring ports to this
crate, not a place to grow session logic).
