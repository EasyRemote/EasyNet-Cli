# RemoteApp Input Geometry Revision Gate

## Intent

Close the execution-path seam where pointer input could be computed from an older client-visible target geometry than the daemon's current target tracker snapshot.

## Invariants

- Media, target events, frontend pointer frames, and daemon input transforms must share a target geometry revision when target-local geometry exists.
- The daemon must reject stale pointer frames before OS input injection.
- Display-global input can omit a target geometry revision because it does not use a target-local pointer transform.
- This change is a safety gate, not proof of product-level input injection.

## Implementation

- Add optional `target_geometry_revision` to pointer input frames.
- Reject pointer frames with missing or mismatched geometry revision when the effective input policy carries a pointer target revision.
- Include the session `targetTracking.targetGeometryRevision` in frontend pointer frames.
- Extend lifecycle/input and frontend boundary gates to pin this contract.

## Verification

- `cargo test -p easynet-plugin-remote-desktop pointer_input_rejects_stale_target_geometry_revision_before_os_injection --lib`
- `bash tools/scripts/check-remoteapp-lifecycle-input-boundary.sh`
- `bash tests/scripts/test_check_remoteapp_lifecycle_input_boundary.sh`
- `bash tools/scripts/check-remoteapp-frontend-invocation-boundary.sh`
- `bash tests/scripts/test_check_remoteapp_frontend_invocation_boundary.sh`
