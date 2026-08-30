# RemoteApp media pipeline evidence

## Intent

Close a runtime evidence seam in the RemoteApp production media path. The
macOS ScreenCaptureKit + VideoToolbox + WebRTC path already emits low-level
encoder/WebRTC diagnostics, but the `MEDIA_PIPELINE_STATS` event does not yet
project stable product evidence fields that a live E2E runner can bind across
baseline, degraded-network, and backpressure scenarios.

This change does not claim full media product completion. Host audio and live
cross-device evidence remain required blockers. The goal is to make the real
native data path emit canonical, session-bound video pipeline evidence instead
of forcing verifiers to reverse-engineer product semantics from debug dumps.

## Invariants

- Invocation, descriptor, authority, and receipt semantics remain unchanged.
- The RemoteApp plugin remains the `AbilityImpl`; the Device remains substrate,
  not callee identity.
- `MEDIA_PIPELINE_STATS` remains a session event projected by the daemon-owned
  RemoteApp session store.
- Every media stats sample must bind:
  - selected Resource URA;
  - session id;
  - transport epoch;
  - media source epoch;
  - stable media pipeline id;
  - negotiated video codec / payload / transport;
  - requested/effective/measured FPS;
  - target/observed bitrate;
  - bounded queue and stale-frame drop policy;
  - adaptation/backpressure/drop events when observed.
- Host audio is reported as not implemented; this change must not fake audio
  readiness or product completion.

## Architecture

The correct layer is `plugins/remote-desktop/src/transport/webrtc_native_media.rs`
because it owns the real native media loop and has direct access to encoder
counters, WebRTC stats, bitrate controller state, and target binding context.

The session/event layer already wraps stats with target binding context, so the
native loop should supply a product-shaped stats payload rather than pushing
policy into frontend verifiers.

## Execution checklist

- Add a small typed stats projection object for the native WebRTC media loop.
- Emit `remoteapp_media_pipeline_stats_v1` fields on periodic and terminal
  stats samples.
- Emit per-sample adaptation events for bitrate shifts, frame drops, and sender
  backpressure, bound to Resource/session/pipeline/epoch.
- Add a focused unit test for the stats projection contract.
- Update the product readiness audit/matrix and closure audit gate.

## Verification

- `cargo test --features axon-pb --lib remote_desktop::transport::webrtc_native_media`
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh`
- `bash tools/scripts/remoteapp-media-adaptation-e2e.sh --self-test`
- `rustfmt --edition 2021 --check plugins/remote-desktop/src/transport/webrtc_native_media.rs`
- `git diff --check`

## Verification results

- PASS — `cargo test --features axon-pb --lib remote_desktop::transport::webrtc_native_media`
  - 2 passed; 6117 filtered out.
- PASS — `bash tools/scripts/check-remoteapp-product-closure-audit.sh`
- PASS — `bash tools/scripts/remoteapp-media-adaptation-e2e.sh --self-test`
- PASS — `rustfmt --edition 2021 --check plugins/remote-desktop/src/transport/webrtc_native_media.rs`
- PASS — `git diff --check`

## Decisions

- Use the media backend id as the stable `media_pipeline_id`; it represents the
  concrete capture/encode/carrier pipeline selected by `remote_desktop.attach`.
- Keep `audio_codec` null and `host_audio_not_implemented=true` until real host
  audio exists.
- Treat raw encoder/WebRTC stats as nested diagnostics; product gates should
  consume the stable top-level fields.
