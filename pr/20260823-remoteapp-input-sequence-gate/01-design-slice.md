# RemoteApp input sequence gate slice

Date: 2026-08-23

## Product requirement

RemoteApp pointer/keyboard input must be controllable on the execution path,
not only shaped by frontend data structures. Once a direct WebRTC input channel
is open, the daemon must reject replayed or out-of-order client input frames
before OS injection.

## Boundary

- Plugin input plane owns data-channel frame ordering and local input
  execution.
- Session aggregate still owns lifecycle, transport epoch, target readiness,
  and terminal state.
- Axon Invocation is not used for high-frequency pointer/key frames after the
  data channel is negotiated.

## Implemented slice

- Add a per-channel monotonic `client_sequence` gate in the RemoteApp input
  loop.
- Reject frames whose `client_sequence` is not greater than the last observed
  sequence on that channel.
- Preserve existing optional sequence compatibility for diagnostic callers that
  do not yet send `client_sequence`.
- Keep stale/out-of-order input as a normal `INPUT_FRAME_REJECTED` diagnostic
  without closing the media/control transport.

## Non-claims

This does not complete product-grade input. Remaining blockers include live OS
focus validation, cross-platform input backends, app/window target-scoped input
execution, latency artifacts, and visible E2E proof of observed pointer/key
effects.
