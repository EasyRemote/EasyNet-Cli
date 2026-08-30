# Decisions and evidence

## Decision

Add `tools/scripts/remoteapp-cross-device-remoteapp-e2e.sh` as the artifact
contract for real cross-device RemoteApp sessions. The verifier validates
externally collected evidence instead of inventing a daemon bypass.

Add `EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_REMOTEAPP_REPORT_JSON` to
`tools/scripts/remoteapp-product-completion-e2e.sh`. The existing synthetic
cross-device smoke is still required, but it is no longer the only
cross-device-related input to the product-complete claim.

## Verification plan

- `bash -n tools/scripts/remoteapp-cross-device-remoteapp-e2e.sh tests/scripts/test_remoteapp_cross_device_remoteapp_e2e.sh tools/scripts/remoteapp-product-completion-e2e.sh tests/scripts/test_remoteapp_product_completion_e2e.sh`
- `bash tools/scripts/remoteapp-cross-device-remoteapp-e2e.sh --self-test`
- `bash tests/scripts/test_remoteapp_cross_device_remoteapp_e2e.sh`
- `bash tools/scripts/remoteapp-product-completion-e2e.sh --self-test`
- `bash tests/scripts/test_remoteapp_product_completion_e2e.sh`
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh`
- `bash tests/scripts/test_check_remoteapp_product_closure_audit.sh`
- `git diff --check -- tools/scripts/remoteapp-cross-device-remoteapp-e2e.sh tests/scripts/test_remoteapp_cross_device_remoteapp_e2e.sh tools/scripts/remoteapp-product-completion-e2e.sh tests/scripts/test_remoteapp_product_completion_e2e.sh pr/20260823-remoteapp-cross-device-remoteapp-evidence/00-intent.md pr/20260823-remoteapp-cross-device-remoteapp-evidence/01-invariants.md pr/20260823-remoteapp-cross-device-remoteapp-evidence/02-decisions-and-evidence.md`

## Verification results

- `bash -n tools/scripts/remoteapp-cross-device-remoteapp-e2e.sh tests/scripts/test_remoteapp_cross_device_remoteapp_e2e.sh tools/scripts/remoteapp-product-completion-e2e.sh tests/scripts/test_remoteapp_product_completion_e2e.sh`
- `git diff --check -- tools/scripts/remoteapp-cross-device-remoteapp-e2e.sh tests/scripts/test_remoteapp_cross_device_remoteapp_e2e.sh tools/scripts/remoteapp-product-completion-e2e.sh tests/scripts/test_remoteapp_product_completion_e2e.sh pr/20260823-remoteapp-cross-device-remoteapp-evidence/00-intent.md pr/20260823-remoteapp-cross-device-remoteapp-evidence/01-invariants.md pr/20260823-remoteapp-cross-device-remoteapp-evidence/02-decisions-and-evidence.md`
- `bash tools/scripts/remoteapp-cross-device-remoteapp-e2e.sh --self-test`
- `bash tests/scripts/test_remoteapp_cross_device_remoteapp_e2e.sh`
- `bash tools/scripts/remoteapp-product-completion-e2e.sh --self-test`
- `bash tests/scripts/test_remoteapp_product_completion_e2e.sh`
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh`
- `bash tests/scripts/test_check_remoteapp_product_closure_audit.sh`
