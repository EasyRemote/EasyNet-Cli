# Boundary proof

## Root abstraction problem

The FFI invocation registry is the owner of stream and bidi session lifecycle.
It currently treats unknown stream/bidi identifiers as successful terminal
operations for `cancel`, `close`, and `bidi_close`. That collapses two different
states:

- a registered resource that reaches a canonical terminal transition; and
- a caller-supplied identifier that is not owned by the current runtime handle.

Those states must not share `RUNTIME_OK`, because product callers can otherwise
lose the only observable evidence that a stream/session was never part of the
runtime lifecycle.

## Owning boundary

The invariant belongs in `src/ffi/invocation/mod.rs`, where runtime handles,
stream ids, bidi ids, and provider cancellation controls are registered and
removed. SDK facades must not implement their own repair logic for unknown
resources.

## Canonical invariant

- A live runtime handle plus an unknown stream id returns `ERR_INVALID_HANDLE`.
- A live runtime handle plus an unknown bidi id returns `ERR_INVALID_HANDLE`.
- A cross-handle stream/bidi operation remains `ERR_INVALID_HANDLE`.
- Cancellation idempotency is preserved only for registered resources through
  `ProviderCancellationControl`.
- Half-close idempotency is preserved only for registered bidi sessions through
  `reserve_close_send_frame`.

## Compatibility removed

The legacy "unknown means already closed" path is removed from the FFI ABI
implementation and from tests. This is an intentional convergence change: a
missing resource is invalid lifecycle state, not a successful terminal state.
