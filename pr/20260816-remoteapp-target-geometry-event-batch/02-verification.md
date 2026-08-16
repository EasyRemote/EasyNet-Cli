# RemoteApp target geometry event batch verification

## Checks

- `cargo test daemon::plugins::remote_desktop::target_tracking::tests::`
  - passed: 15 tests, 0 failures
- `cargo test daemon::plugins::remote_desktop::session::tests::`
  - passed: 18 tests, 0 failures
- `bash tools/scripts/check-remoteapp-lifecycle-input-boundary.sh`
  - passed
- `bash tests/scripts/test_check_remoteapp_lifecycle_input_boundary.sh`
  - passed: `test_check_remoteapp_lifecycle_input_boundary.sh: all cases passed`
- EasyNet Frontend targeted remote desktop tests:
  - command:
    `npm test -- --run src/lib/api/remote-desktop-protocol.test.ts src/store/media-channel-store.test.ts src/store/media-channel-invocation.test.ts src/components/easynet/DeviceMediaAccess.test.tsx`
  - passed: 4 files, 50 tests
- `git diff --check`
  - passed
- `codegraph sync . && codegraph status .`
  - passed, index up to date after syncing 2 changed Rust files
- URA-only touched-file scan:
  - command:
    `rg -n "\bURI\b|\buri\b" plugins/remote-desktop/src/target_tracking.rs plugins/remote-desktop/src/session.rs tools/scripts/check-remoteapp-lifecycle-input-boundary.sh tests/scripts/test_check_remoteapp_lifecycle_input_boundary.sh pr/20260816-remoteapp-target-geometry-event-batch || true`
  - no matches

## Boundary evidence

- `RemoteAppTargetBindingStateMachine` still commits one target snapshot update
  per `TargetObservation::GeometryChanged`.
- `TargetTrackingEmission` expands that committed update into one or more
  ordered lifecycle event records.
- `RemoteDesktopSession::push_target_tracking_event` writes every ordered
  target event into the bounded session event log, where sequence numbers are
  assigned by `RemoteDesktopEventLog`.
- Combined move+resize observations now produce `TARGET_MOVED` then
  `TARGET_RESIZED` with the same committed `target_geometry_revision`.
