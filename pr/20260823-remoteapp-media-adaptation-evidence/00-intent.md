# RemoteApp media adaptation evidence gate intent

RemoteApp interactive desktop product readiness requires a real audio/video
data-plane proof, not only a codec implementation, static descriptor, frontend
stats widget, or synthetic media carrier.

This batch adds a runner-agnostic verifier for live RemoteApp media artifacts.
It keeps the architecture boundary intact:

- public session lifecycle remains `remote_desktop.*` Ability invocation;
- display/window/application Resource URA remains the Invocation subject;
- WebRTC/native/raw media remains session transport, not a second Invocation
  model;
- the RemoteDesktop plugin owns native capture/encode/transport execution;
- terminal receipts remain visible for session closure.

The verifier is intentionally strict. Product-level media evidence must prove
codec negotiation, video frame flow, host audio, actual FPS, bitrate telemetry,
 adaptive behavior under impaired conditions, bounded queue/backpressure, stale
frame drop policy, rendered media, and terminal receipts.

Self-test evidence only proves the verifier contract. It is not product
readiness evidence.
