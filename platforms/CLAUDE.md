# platforms — shell contract

A shell does exactly four kinds of work, and nothing else:

1. Translate OS events into neutral input the core understands.
2. Provide a surface handle the renderer draws onto.
3. Implement the ports the core needs from the host — clipboard, IME, outbox
   persistence, clock.
4. Render product UI (toolbars, modals, presence chrome).

If you are writing a fifth kind of code — coordinate math, op construction,
collaboration rules — stop; it belongs in `crates/scene-core`,
`crates/client-runtime`, or `crates/renderer-core`. A shell decides nothing
about the canvas; it forwards input and renders what the core hands back.

Adding a new OS shell means implementing those ports against
`crates/platform-contract` with **zero** core changes. If a new shell forces a
core change, the contract is wrong — fix the contract, not the shell.

Each empty sibling directory (`macos/`, `ios/`, `android/`) is a reserved shell
target, not scaffolding to flesh out speculatively. `web/` is the only live
shell; its own `CLAUDE.md` carries the web-specific rules.
