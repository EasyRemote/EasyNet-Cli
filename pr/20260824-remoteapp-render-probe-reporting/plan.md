# RemoteApp render probe reporting

## Intent

Close the media evidence seam between browser presentation and Runtime session
state. The native media loop emits `remoteapp_media_pipeline_stats_v1`, and the
frontend can display it, but `remote_desktop.report_client_state` only accepts
`browser_stats`. Live product evidence for media adaptation also needs a
render-probe object bound to Resource/session/pipeline/codec/transport.

This change adds the Runtime-side contract for accepting bounded render-probe
evidence. It does not fake decoded payload fingerprints or host audio; those
must come from a real browser/Tauri/live media runner when available.

## Invariants

- Invocation, authority, descriptor, and receipt semantics are unchanged.
- `remote_desktop.report_client_state` remains the public session ability for
  browser-observed presentation evidence.
- Render-probe fields are bounded and schema-governed.
- Render-probe evidence is merged into session media stats and replayed through
  `MEDIA_PIPELINE_STATS`.
- Host audio remains unsupported unless a real runner provides audio evidence.

## Architecture

Layer order:

1. Frontend/browser observes media presentation.
2. `remote_desktop.report_client_state` carries bounded client evidence.
3. Runtime session store merges evidence into `media_stats`.
4. `watch_events` replays target-bound `MEDIA_PIPELINE_STATS`.

This task changes step 2 and validates step 3.

## Checklist

- Extend `report_client_state` input schema with `render_probe`.
- Normalize/copy bounded render-probe fields in the handler.
- Test that render-probe evidence merges with existing media pipeline stats.
- Keep closure/readiness status partial; this is plumbing for stronger live E2E.

## Verification

- `cargo test --features axon-pb --lib report_client -- --nocapture`
- `cargo test --features axon-pb --lib client_media_report_merges_transport_evidence_without_overwriting_device_stats -- --nocapture`
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh`
- `rustfmt --edition 2021 --check plugins/remote-desktop/src/handlers/report_client_state.rs plugins/remote-desktop/src/schema.rs plugins/remote-desktop/src/session.rs`
- `git diff --check`

## Verification results

- FAIL — `cargo test --features axon-pb --lib remote_desktop::handlers::report_client_state remote_desktop::session::tests::report_client_media_state_merges_browser_transport_evidence`
  - Reason: cargo accepts one test filter before `--`; rerun as focused filters.
- PASS — `cargo test --features axon-pb --lib report_client -- --nocapture`
  - 3 passed; 6116 filtered out.
- PASS — `cargo test --features axon-pb --lib client_media_report_merges_transport_evidence_without_overwriting_device_stats -- --nocapture`
  - 1 passed; 6118 filtered out.
- PASS — `bash tools/scripts/check-remoteapp-product-closure-audit.sh`
- PASS — `rustfmt --edition 2021 --check plugins/remote-desktop/src/handlers/report_client_state.rs plugins/remote-desktop/src/schema.rs plugins/remote-desktop/src/session.rs`
- PASS — `git diff --check`
