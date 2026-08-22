# Decisions and evidence

## Decision

Expose `metadata.media_pipeline_support` from the RemoteApp device capability
view.

The projection carries:

- video backend, codec, payload content type, transport, FPS, bitrate, queue,
  stale-frame drop, backpressure, and adaptation policy;
- audio unsupported state from the canonical `audio_support_view`;
- full audio/video product blockers.

## Evidence target

- main-crate unit test pins the projection shape;
- performance boundary script requires the projection and its regression test;
- product-readiness audit/matrix record that this is product transparency, not
  host-audio or degraded-network E2E completion.
