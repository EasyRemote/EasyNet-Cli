# SDK-Owned EasyRemote Bidi Session

## Objective

Move EasyRemote bidirectional session lifecycle glue into the EasyNet-Cli
Python SDK. EasyRemote should keep only product-facing method names and public
error mapping; it should not own daemon bidi close/cancel/timeout/wire-error
semantics.

## Boundary Proof

- Axon remains the source of stream/bidi protocol invariants.
- EasyNet-Cli Runtime Core owns the `BidiSession` state machine and
  `DaemonBidiChannel` projection.
- The Python SDK owns the EasyRemote-facing bidi adapter because it is a
  language facade over daemon Runtime Core, not product behavior.
- EasyRemote owns only product ergonomics: `send`, `recv`, `close`, `cancel`,
  context manager shape, and mapping SDK errors into EasyRemote errors.

## Invariants

- `close()` releases an open EasyRemote session deterministically by cancelling
  before closing when the underlying Runtime Core session is not terminal.
- `cancel()` remains distinct from `close()` and does not pretend to be a local
  half-close.
- `recv(timeout=...)` maps client wait expiry to a typed SDK timeout with
  `reason=client_wait_timeout`.
- Remote bidi wire errors surface as typed SDK errors instead of raw frame
  payloads.
- EasyRemote no longer maintains a private bidi channel wrapper.

## Non-Goals

- Do not change the daemon SDK requirements spec.
- Do not implement new Axon bidi protocol states in Python.
- Do not change EasyRemote's public `BidiSession` method names.
