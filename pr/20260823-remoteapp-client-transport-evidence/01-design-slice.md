# RemoteApp client-observed transport evidence

## Product seam

The frontend and the network fallback verifier both need selected WebRTC
candidate-pair evidence. The daemon already projects native device-side
`media_stats.webrtc_stats.selected_candidate_pair`, and the frontend parser can
render it. But `remote_desktop.report_client_state` only accepts
`state + transport_epoch`, so browser-observed selected-pair and decode stats
cannot enter the session projection through the governed ability path.

## Invariants

- Client transport evidence is observational data, not a new lifecycle
  authority. `state` still drives the existing client media state machine.
- Evidence must be scoped to the active `transport_epoch`; stale epochs must be
  rejected by the existing session control path.
- The report schema must stay bounded and reject arbitrary JSON bags.
- Selected candidate pair projection must expose both `id` and
  `candidate_pair_id` so existing frontend parsing and network E2E evidence use
  one canonical session fact.
- Browser decode stats must be persisted under `media_stats.browser_stats` so
  existing frontend media stats projection can render them after show/session
  refresh.

## Expected impact

The Browser/Tauri RemoteApp path can report real selected candidate-pair and
render/decode stats back to the daemon. Session views then retain those facts
as epoch-bound evidence, improving NAT/relay and media-quality product
verification without letting the browser bypass daemon-owned lifecycle,
authority, or receipt semantics.

## Implementation notes

- `remote_desktop.report_client_state` now accepts bounded `client_transport`
  and `browser_stats` report objects.
- The daemon normalizes selected candidate-pair identity to both `id` and
  `candidate_pair_id`, rejects unbounded strings, and merges client evidence
  into existing `media_stats` for the active transport epoch.
- Client reports do not become a new lifecycle source of truth. The session
  still rejects stale epochs and lifecycle transitions remain daemon-owned.
- Frontend boundary checks now require the Browser/Tauri path to collect
  RTCPeerConnection stats and submit selected-pair/browser stats through the
  governed ability path.

## Verification

- `rustfmt --edition 2021 --check plugins/remote-desktop/src/handlers/report_client_state.rs plugins/remote-desktop/src/schema.rs plugins/remote-desktop/src/session.rs plugins/remote-desktop/src/session_store.rs plugins/remote-desktop/src/session_transport_state.rs plugins/remote-desktop/src/target_observer.rs plugins/remote-desktop/src/view.rs plugins/remote-desktop/src/view_transport.rs`
- `cargo test -p easynet --features axon-pb client_transport_evidence_is_bounded_and_canonicalized -- --nocapture`
- `cargo test -p easynet --features axon-pb client_transport_evidence_rejects_unbounded_string_fields -- --nocapture`
- `cargo test -p easynet --features axon-pb client_media_report_merges_transport_evidence_without_overwriting_device_stats -- --nocapture`
- `bash tools/scripts/check-remoteapp-frontend-invocation-boundary.sh`
- `bash tests/scripts/test_check_remoteapp_frontend_invocation_boundary.sh`
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh`

Note: an earlier verification command started three cargo tests in parallel and
two duplicate processes were interrupted to avoid Cargo lock contention. The
same tests were then run serially and passed.
