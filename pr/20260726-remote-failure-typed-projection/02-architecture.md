# Architecture

## Layering

- Presence/session carriers produce `SessionFailure`.
- Remote dispatch projects `SessionFailure` into transport status.
- Raw error strings are transport diagnostics and do not own runtime semantics.

## Module boundary

`src/daemon/invocation/dispatch/remote_failure.rs` owns the projection from remote failure facts to gRPC `Status`.

The route resolver and admission gate continue to produce typed status/failure detail. The projection layer does not probe keyrings, directories, routes, or descriptors.

## Ownership rule

Compatibility with untyped remote failures is not part of the canonical runtime model. Untyped failures fail closed as upstream transport failure rather than being guessed by substring.
