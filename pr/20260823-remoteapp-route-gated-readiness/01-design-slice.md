# RemoteApp route-gated production readiness

## Product seam

`RemoteDesktopTransportView` already computes route-gated
`transport.production_ready = production_media_ready && production_route_ready`.
However the public `production_readiness.ready` field only used
`session.production_media_ready()`. Because the frontend online predicate is
required to derive from `production_readiness.ready`, a host-only/no NAT-relay
session could be rendered as product-online while the transport projection
correctly said it was not production-ready.

## Invariants

- `production_media_ready` remains the video/media predicate: target scope,
  production codec, device sender, and client presentation.
- `production_readiness.ready` is the product predicate consumed by the UI and
  must include route readiness through `RemoteDesktopTransportView`.
- `production_readiness.blocked_reason` must remain centralized in the session
  view projection and must distinguish route blockers after media is otherwise
  ready.
- `route_readiness_blocker` remains a separate structured object so the UI can
  present retry/recovery details without inferring policy from candidate rows.
- This slice does not claim NAT/relay/WebRTC fallback completion; it prevents a
  false online state until live route evidence exists.

## Expected impact

Host-only or STUN-without-relay sessions can still expose media/client progress,
but their public product readiness remains false with a typed route blocker.
Relay-backed sessions keep reporting product readiness once media, client
presentation, and route readiness are all true.

## Verification

- Initial focused Rust failure:
  `cargo test -p easynet --features axon-pb production_media_ready_requires_production_codec_and_sender_ready -- --nocapture`
  failed because the test expected a route blocker before creating explicit
  host-only route evidence.
- Passed:
  `cargo test -p easynet --features axon-pb production_media_ready_requires_production_codec_and_sender_ready -- --nocapture`
- Passed:
  `cargo test -p easynet --features axon-pb host_only_route_keeps_production_offline_after_client_media_presents -- --nocapture`
- Passed:
  `bash tools/scripts/check-remoteapp-frontend-invocation-boundary.sh`
- Passed:
  `bash tests/scripts/test_check_remoteapp_frontend_invocation_boundary.sh`
- Passed:
  `bash tools/scripts/check-remoteapp-lifecycle-input-boundary.sh`
- Passed:
  `bash tests/scripts/test_check_remoteapp_lifecycle_input_boundary.sh`
- Passed:
  `bash tools/scripts/check-remoteapp-product-closure-audit.sh`
