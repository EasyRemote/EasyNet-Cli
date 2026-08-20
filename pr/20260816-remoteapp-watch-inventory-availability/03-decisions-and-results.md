# Remoteapp Watch Inventory Availability Decisions and Results

## Decisions

1. Model target discovery outage as an inventory-state transition, not as target removal.
2. Publish a typed `target_inventory_unavailable` watch event when `screen_target_discovery_available=false`.
3. Keep `removed_resource_uras` empty for unavailable observations because the daemon has not proven that any prior target disappeared.
4. Include discovery availability in the stable inventory hash so available-empty and unavailable-empty snapshots remain distinguishable.
5. Keep freshness-only timestamp changes excluded from the inventory hash.
6. Extend both performance and picker-subject boundary gates because frontend target selection depends on the same inventory identity semantics.

## Results

- `cargo fmt --all` passed.
- `cargo test -q -p easynet --features remote-desktop,headless-media watch_remote_targets --lib` passed 11 tests.
- `cargo test -q -p easynet --features remote-desktop,headless-media remote_desktop --lib` passed 318 tests.
- `bash tools/scripts/check-remoteapp-performance-boundary.sh` passed.
- `bash tests/scripts/test_check_remoteapp_performance_boundary.sh` passed all mutation cases.
- `bash tools/scripts/check-remoteapp-picker-subject-boundary.sh` passed.
- `bash tests/scripts/test_check_remoteapp_picker_subject_boundary.sh` passed all mutation cases.
- `bash tools/scripts/check-remoteapp-e2e-acceptance-boundary.sh` passed.
- `bash tools/scripts/check-remoteapp-target-binding-boundary.sh` passed.
- `bash tools/scripts/check-remoteapp-frontend-invocation-boundary.sh` passed.
- `cargo test -q -p easynet --features remote-desktop,headless-media remoteapp_ --test script_checks` passed 7 tests.
- EasyNet Frontend `npm test -- --run src/lib/api/remote-desktop-protocol.test.ts src/store/media-channel-store.test.ts src/store/media-channel-invocation.test.ts src/components/easynet/DeviceMediaAccess.test.tsx` passed 4 files and 50 tests.

## Boundary note

EasyNet Frontend remained read-only in this iteration because its worktree already contains broad unrelated modified and untracked files. The CLI daemon contract now emits enough typed evidence for frontend consumers to render a retry/unavailable inventory state without inventing removals.
