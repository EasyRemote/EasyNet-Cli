# RemoteApp rehydrate event binding

## Product seam

Crash/restart recovery evidence needs lifecycle events that are bound to the
selected Resource URA and target identity. `SESSION_REHYDRATED` was emitted as
an ad-hoc payload with recovery metadata but without the target binding fields
that the event log promotes to top-level searchable evidence.

That leaves live crash/restart artifacts weaker than the verifier contract:
the event proves a session was rehydrated, but not which selected Resource,
binding epoch, geometry revision, media source epoch, and consent epoch were
rehydrated.

## Invariants

- Rehydration remains daemon-owned startup recovery, not a frontend lifecycle
  decision.
- Non-terminal sessions emit exactly one `SESSION_REHYDRATED` event during
  snapshot-to-session recovery.
- The event payload must carry the same target identity fields used by target
  resolution/binding events so event-log top-level projections can bind
  recovery evidence to the selected Resource.
- Terminal recovery snapshots must not emit a non-terminal rehydrate event.

## Implementation notes

- Add a typed `session_events::session_rehydrated` projection.
- Replace the session aggregate's ad-hoc JSON with the typed projection.
- Strengthen runtime/session tests to require Resource/target binding fields in
  show_session and watch_events replay.

## Verification

- `rustfmt --edition 2021 --check plugins/remote-desktop/src/session_events.rs plugins/remote-desktop/src/session.rs plugins/remote-desktop/src/runtime.rs`
- `cargo test -p easynet --features axon-pb rehydrated_non_terminal_session_can_start_new_media_epoch_without_new_session -- --nocapture`
- `cargo test -p easynet --features axon-pb plugin_startup_rehydrates_recovery_snapshot_for_public_show_session -- --nocapture`
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh`

Note: one duplicate cargo test process was interrupted to avoid artifact-lock
contention; the affected runtime startup test was rerun serially and passed.
