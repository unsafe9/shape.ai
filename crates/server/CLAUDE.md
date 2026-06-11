# server — the native transport/persistence seam

The tokio/axum platform layer in front of the pure cores. It orchestrates
transport, persistence, and fan-out; it never reimplements canvas logic.
Op-apply, layout, hit testing, and export all live in `scene-core` — this crate
CALLS into them. If you find yourself authoring scene behavior here, stop and
move it into a core crate.

Never add a dependency to a sibling pure crate (`scene-core`, `storage-core`,
…). They are depended on by path only, so their `wasm32-unknown-unknown` builds
stay intact; a server-only dep leaking into one breaks the wasm shell.

Identity is `userId`-only. Every future auth attach point is marked
`TODO(auth)` — keep that marker on any new spot where authn/authz would attach;
do not invent an auth model.

Ambient time, randomness, threads, and IO are acceptable here — this is the
platform layer where the pure cores get their injected seams sourced. Source the
clock at this boundary and pass it down; never let a core crate acquire one.
