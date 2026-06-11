# platforms/web — the web shell

The four shell jobs (see `../CLAUDE.md`), realized in TypeScript + Svelte.

Allowed here: DOM/pointer/keyboard event translation, IME composition,
clipboard, and the injected seams the wasm core needs from a browser — timers
and id/seq generators handed in at construction. Everything else routes into the
wasm core.

Forbidden: any canvas decision in TS — coordinate math, op construction,
hit-test, layout, undo, or collaboration policy. If you reach for one, stop; it
belongs in a core crate, exposed over `wasm_api` and called from here.

`controller/gestureBindings.ts` is the ONLY sanctioned frozen mirror of core
state: the hot pointer path can't await a wasm round-trip per event, so the
binding token per gesture is duplicated as a constant. Its contract is
`verifyGestureBindings`, which pins the mirror to the runtime gesture catalog in
the unit gate — a drift there fails the build, not the routing. Add no other
TS mirror of catalog data.

`runtime/` is an IO-adapter layer over `crates/client-runtime` (WS transport,
IndexedDB outbox, browser timers driving the engine's decisions). The
collaboration *logic* lives in that crate; it must not reappear in TS. New
session behavior starts in `client-runtime`, and `runtime/` only wires effects
to it.
