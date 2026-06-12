# server — the native transport/persistence seam

The tokio/axum platform layer in front of the pure cores. It orchestrates
transport, persistence, and fan-out; it never reimplements canvas logic.
Op-apply, layout, hit testing, and export all live in `scene-core` — this crate
CALLS into them. If you find yourself authoring scene behavior here, stop and
move it into a core crate.

Stay as stateless as the work allows — the target is scale-out and a future
multi-node real-time canvas where many instances serve one logical canvas. Treat
in-process state as a rebuildable cache over durable storage and the coordination
seam, never the only copy: the per-canvas in-memory scene is evictable and
reconstructable (storage write-through plus the journal tail), so losing an
instance loses no canvas state. Route every cross-instance concern through an
abstracted external-memory seam — persistence through the `storage-core` adapter,
shared lease/presence/pub-sub through the `coordination` crate's `Coordinator` —
so today's single-node backends swap for an external store (e.g. Redis) by adding
an adapter, not by editing server logic. Never add process-local state a second
instance cannot see.

Never add a dependency to a sibling pure crate (`scene-core`, `storage-core`,
…). They are depended on by path only, so their `wasm32-unknown-unknown` builds
stay intact; a server-only dep leaking into one breaks the wasm shell.

Identity is `userId`-only. Every future auth attach point is marked
`TODO(auth)` — keep that marker on any new spot where authn/authz would attach;
do not invent an auth model.

Ambient time, randomness, threads, and IO are acceptable here — this is the
platform layer where the pure cores get their injected seams sourced. Source the
clock at this boundary and pass it down; never let a core crate acquire one.
