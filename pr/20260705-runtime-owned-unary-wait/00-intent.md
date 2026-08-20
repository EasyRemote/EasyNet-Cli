# Runtime-Owned EasyRemote Unary Wait

## Objective

Move EasyRemote unary wait/timeout/retired-transport lifecycle ownership from
EasyRemote product code into the Python SDK Runtime Core adapter. EasyRemote
should keep product ergonomics and Invocation tuple preparation only; SDK
Runtime Core should own daemon transport reuse, bounded wait, close, and
retirement semantics.

## Boundary

- SDK owns daemon transport lifecycle state, timeout-to-retire transitions, and
  delayed close of an active unary transport.
- EasyRemote owns target resolution, argument lowering, public exception
  mapping, and product method names.
- The state machine must be single-flight per transport pool and deterministic:
  idle close closes immediately; active close retires the current transport
  without blocking; the pool can reconnect with a fresh owned transport after a
  close; timed-out active calls retire the transport and prevent reuse.
- A queued caller wait timeout must not retire the active transport because it
  has not yet been dispatched to the daemon.
- The implementation must not add cancellation claims. Client-side timeout only
  bounds the caller wait; daemon execution remains governed by the ability
  timeout and may finish later.

## Non-goals

- Do not change `docs/spec/daemon-sdk-requirements-v1.md`.
- Do not change EasyRemote public `Client.call/execute/invoke` behavior.
- Do not implement stream/bidi live-tail or cancellation semantics in this
  slice.
