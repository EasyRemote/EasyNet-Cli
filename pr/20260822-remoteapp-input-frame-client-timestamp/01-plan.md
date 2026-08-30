# Plan — RemoteApp Input Frame Client Timestamp Schema

## Product invariant

The frontend input frame shape and daemon input frame parser must agree. A
browser frame that passes frontend input gating must not be rejected by the
daemon merely because the frontend attaches client-side metadata used for input
observability.

## Boundary

- The RemoteApp plugin owns the WebRTC input data-channel frame schema.
- The EasyNet frontend may attach client-side send metadata before writing to
  the data channel.
- Axon Invocation semantics are unchanged; high-frequency input remains on the
  negotiated RemoteApp data channel after session setup.

## Current seam

`rdSendInput` sends `{ ...frame, sent_at_ms: Date.now() }`, while
`PointerInputFrame` and `KeyInputFrame` use `#[serde(deny_unknown_fields)]`
without `sent_at_ms`. The daemon therefore parses real frontend pointer/key
frames as invalid input frames before policy or OS injection can run.

## Required change

1. Add optional `sent_at_ms` to pointer and key input frames.
2. Validate the timestamp is finite enough for JavaScript millisecond epoch
   values without making it mandatory.
3. Preserve strict rejection for genuinely unknown fields.
4. Include client timestamp evidence in accepted/rejected input events when
   present.
5. Add tests and boundary gates so frontend/daemon schema drift is caught.

## Product effect

Frontend pointer/key input can reach daemon policy and OS-injection decisions
instead of being rejected at JSON schema parsing. This is required before any
real input latency or injection E2E evidence can be trusted.
