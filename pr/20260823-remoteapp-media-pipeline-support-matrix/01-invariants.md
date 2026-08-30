# Invariants

1. Video readiness and full audio/video product readiness are separate states.
2. Host audio remains explicitly unsupported until capture, codec negotiation,
   WebRTC transport, frontend playback, and E2E evidence exist.
3. Bounded queues, stale-frame drop, backpressure, and bitrate adaptation must
   be visible as product metadata when they are part of the runtime path.
4. Capability metadata is not live E2E evidence. The projection must say so
   directly so UI and release gates cannot treat source presence as product
   closure.
5. Diagnostic display streaming must not imply window/application production
   support.
