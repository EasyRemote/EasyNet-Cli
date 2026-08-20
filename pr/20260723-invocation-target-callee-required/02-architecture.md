# Architecture

`invocation_wire.rs` owns protobuf tuple extraction for daemon dispatchers. A single helper should express the target-routing invariant:

- `callee_ura_from_envelope`: required callee field, URA grammar validation.

Dispatchers consume that helper instead of locally reinterpreting caller/callee shape. This keeps route selection cohesive and prevents mode-specific fallback behavior.
