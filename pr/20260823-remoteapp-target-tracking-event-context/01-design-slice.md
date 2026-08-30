# RemoteApp target-tracking event context

## Intent

Close one RemoteApp product seam: target lifecycle events emitted by the
target-tracking state machine must carry the same selected-target evidence as
session-level projected events. A frontend cannot safely drive app/window
interaction from resize, rebind, lost, topology, or permission events if those
events expose only scalar epochs without the canonical target binding, scope
audit, and latest target diagnostic.

## Boundary

- Runtime/plugin layer: `plugins/remote-desktop`.
- No Axon protocol changes.
- No frontend behavior change in this slice.
- No changes to unrelated invocation dispatcher files currently dirty in the
  checkout.

## Invariants

1. Target-tracking events remain owned by `RemoteAppTargetBindingStateMachine`.
2. Session event log continues to add transport/session envelope metadata.
3. Every target lifecycle event payload must include:
   - `subject_ura`
   - `binding_id`
   - `binding_epoch`
   - `target_identity_epoch`
   - `target_geometry_revision`
   - `media_source_epoch`
   - `consent_epoch`
   - `target_binding`
   - `scope_audit`
   - `latest_target_diagnostic`
4. Pending rebind evidence must remain explicit; adding current binding context
   must not erase pending binding/media-source epochs.
5. This is target-context projection only. It does not certify full RemoteApp
   product completion.

## Verification

- PASS:
  `rustfmt --edition 2021 --check plugins/remote-desktop/src/target.rs plugins/remote-desktop/src/target_tracking.rs plugins/remote-desktop/src/session.rs`
- PASS:
  `cargo test -p easynet --features axon-pb target_tracking_events_include_active_transport_epoch_at_session_boundary -- --nocapture`
- PASS:
  `cargo test -p easynet --features axon-pb pending_media_rebind_failure_rejects_session_rebinding -- --nocapture`
- PASS:
  `cargo test -p easynet --features axon-pb target_tracking -- --nocapture`
- PASS:
  `bash tools/scripts/check-remoteapp-product-closure-audit.sh`

## Decision

Target lifecycle events now enrich payloads in the target-tracking state
machine, not in the frontend. `RemoteAppTargetBinding` exposes a tracker-state
projection so resize/move events carry the current geometry revision while the
committed binding still owns subject, scope audit, consent, and media source
authority. Pending rebind fields remain separate and are not treated as a
committed target.
