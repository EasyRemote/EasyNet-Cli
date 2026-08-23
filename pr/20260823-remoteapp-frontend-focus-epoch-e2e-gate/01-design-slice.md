# RemoteApp frontend lifecycle focus-epoch evidence gate

## Intent

The frontend now sends `target_focus_epoch` on RemoteApp pointer/key frames. The Browser/Tauri lifecycle evidence verifier must also require that fact when an artifact claims interactive input was applied. Otherwise a live UI artifact could still pass with generic input telemetry that does not prove target-scoped freshness.

## Boundary

- EasyNet-Cli daemon remains the authority for target focus epochs and input admission.
- EasyNet frontend may only project and transmit the epoch it received from the daemon session view.
- The Browser/Tauri verifier validates externally produced evidence; it does not simulate the UI or claim product completion.

## Invariants

1. `policy_blocked` remains valid evidence for view-only or blocked input paths.
2. `input_applied` must include a positive `target_focus_epoch`.
3. `input_applied.submitted_frame.target_focus_epoch` must match the claimed focus epoch and data-channel `client_sequence`.
4. `input_applied.applied_event.target_focus_epoch` must match the same focus epoch and bind the created session id.
5. Product readiness remains incomplete until a real Browser/Tauri runner provides live evidence.

## Delta

- Tighten `frontend-remoteapp-browser-lifecycle-e2e.sh` evidence validation.
- Add a regression script proving missing/stale focus-epoch evidence is rejected.
- Update the product readiness matrix/audit and closure audit gate.

## Verification

- `bash -n tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh tests/scripts/test_frontend_remoteapp_browser_lifecycle_e2e.sh tools/scripts/check-remoteapp-product-closure-audit.sh tests/scripts/test_check_remoteapp_product_closure_audit.sh`
  - Passed.
- `bash tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh --self-test`
  - Passed.
- `bash tests/scripts/test_frontend_remoteapp_browser_lifecycle_e2e.sh`
  - Passed.
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh`
  - Passed.
- `bash tests/scripts/test_check_remoteapp_product_closure_audit.sh`
  - Passed.
