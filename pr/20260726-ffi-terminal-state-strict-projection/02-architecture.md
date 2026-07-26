# Architecture

`src/ffi/invocation/mod.rs` is the C ABI projection layer for daemon invocation results. It should expose canonical runtime observations, not repair runtime lifecycle spelling.

Layering:

- Daemon invocation runtime produces verified `InvocationOutcome`.
- `canonical_terminal_phase` maps the already verified terminal state to the FFI handle state machine.
- SDK language bindings consume that FFI projection and should not receive silently normalized retired states.

The correct abstraction is an exact terminal-state projection over the canonical runtime lifecycle.
