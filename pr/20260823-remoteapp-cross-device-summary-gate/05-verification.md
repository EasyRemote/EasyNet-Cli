# Verification

Passing commands:

```bash
bash -n tools/scripts/remoteapp-cross-device-remoteapp-e2e.sh tools/scripts/remoteapp-product-completion-e2e.sh tools/scripts/check-remoteapp-product-closure-audit.sh tests/scripts/test_remoteapp_cross_device_remoteapp_e2e.sh tests/scripts/test_remoteapp_product_completion_e2e.sh tests/scripts/test_check_remoteapp_product_closure_audit.sh
bash tools/scripts/remoteapp-cross-device-remoteapp-e2e.sh --self-test
bash tools/scripts/remoteapp-product-completion-e2e.sh --self-test
bash tests/scripts/test_remoteapp_cross_device_remoteapp_e2e.sh
bash tests/scripts/test_remoteapp_product_completion_e2e.sh
bash tools/scripts/check-remoteapp-product-closure-audit.sh
bash tests/scripts/test_check_remoteapp_product_closure_audit.sh
git diff --check -- tools/scripts/remoteapp-cross-device-remoteapp-e2e.sh tools/scripts/remoteapp-product-completion-e2e.sh tools/scripts/check-remoteapp-product-closure-audit.sh tests/scripts/test_remoteapp_cross_device_remoteapp_e2e.sh tests/scripts/test_remoteapp_product_completion_e2e.sh tests/scripts/test_check_remoteapp_product_closure_audit.sh pr/20260823-remoteapp-cross-device-summary-gate
```

Notes:
- The cross-device verifier test now runs the generated self-test evidence through `--run` before asserting report summaries.
- This verifies the cross-device RemoteApp aggregate evidence seam only; it does not prove full RemoteApp product completion.
