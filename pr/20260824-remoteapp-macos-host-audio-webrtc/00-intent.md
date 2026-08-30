# Intent

## Goal

Deliver the first real RemoteApp host-audio path on macOS by carrying
ScreenCaptureKit system audio through Opus over the existing session-owned
WebRTC peer connection.

## Non-goals

- Do not change Axon Invocation, authority, receipt, or stream semantics.
- Do not claim Windows/Linux host audio support.
- Do not mark RemoteApp product-complete without live media evidence.
- Do not create a second audio session or an ad-hoc raw socket.

## Acceptance criteria

- The macOS production capture stream emits bounded 48 kHz stereo PCM chunks.
- A session-owned Opus encoder emits 20 ms packets to a negotiated WebRTC audio
  sender on the same peer connection as video.
- Audio failure is explicit and does not fabricate readiness.
- Product/session views report the real macOS audio capability and codec.
- Unit/contract tests cover PCM framing, bounded buffering, SDP negotiation,
  lifecycle shutdown, and stats projection.
