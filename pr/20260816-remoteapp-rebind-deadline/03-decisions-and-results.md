# Remoteapp Rebind Deadline Decisions and Results

## Decisions

- `Rebinding` expiry is a target state-machine transition, exposed as `expire_rebind_deadline`.
- Session code projects the transition through a typed `TargetRebindDeadlineExpiration` effect so monitor/observer code does not duplicate lifecycle rules.
- Active media-source loss projection is centralized in the session aggregate and reused by target loss and rebind deadline expiry.
- The observer tick evaluates deadline expiry before polling the platform provider, so no-observation ticks are still bounded.
- The store boundary validates session id, binding id, and binding epoch before applying expiry.

## Results

- Pending media rebinds now terminate as `TARGET_REBIND_FAILED` with `target_stale` and `rebind_window_expired` when the 30 second window expires.
- Post-loss rebind attempts now terminate as `TARGET_REBIND_FAILED` with `explicit_rebind_required` and `rebind_window_expired` when no explicit rebind policy commits before the deadline.
- No new public ability shape or callee ownership model was introduced.
- Remote desktop remains device-native execution; service-owned callee work remains separate for user-owned public abilities such as Pages.

## Verification summary

- `cargo test -q -p easynet --features remote-desktop,headless-media remote_desktop --lib`
- `bash tools/scripts/check-remoteapp-lifecycle-input-boundary.sh`
- `bash tests/scripts/test_check_remoteapp_lifecycle_input_boundary.sh`
- `cargo test -q -p easynet --features remote-desktop,headless-media remoteapp_lifecycle_input_boundary_script_holds --test script_checks`
- `bash tools/scripts/check-remoteapp-target-binding-boundary.sh`
- `bash tools/scripts/check-remoteapp-e2e-acceptance-boundary.sh`
- `bash tools/scripts/check-remoteapp-frontend-invocation-boundary.sh`
- `bash tools/scripts/check-remoteapp-performance-boundary.sh`
- EasyNet frontend remote desktop Vitest suite: 4 files, 50 tests passed.
