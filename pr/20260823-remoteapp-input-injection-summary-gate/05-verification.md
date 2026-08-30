# Verification

Passing commands:

```bash
bash -n tools/scripts/remoteapp-input-injection-e2e.sh tools/scripts/remoteapp-product-completion-e2e.sh tools/scripts/check-remoteapp-product-closure-audit.sh tests/scripts/test_remoteapp_input_injection_e2e.sh tests/scripts/test_remoteapp_product_completion_e2e.sh tests/scripts/test_check_remoteapp_product_closure_audit.sh
bash tools/scripts/remoteapp-input-injection-e2e.sh --self-test
bash tools/scripts/remoteapp-product-completion-e2e.sh --self-test
bash tests/scripts/test_remoteapp_input_injection_e2e.sh
bash tests/scripts/test_remoteapp_product_completion_e2e.sh
bash tools/scripts/check-remoteapp-product-closure-audit.sh
bash tests/scripts/test_check_remoteapp_product_closure_audit.sh
git diff --check -- tools/scripts/remoteapp-input-injection-e2e.sh tools/scripts/remoteapp-product-completion-e2e.sh tools/scripts/check-remoteapp-product-closure-audit.sh tests/scripts/test_remoteapp_input_injection_e2e.sh tests/scripts/test_remoteapp_product_completion_e2e.sh tests/scripts/test_check_remoteapp_product_closure_audit.sh pr/20260823-remoteapp-input-injection-summary-gate
```

Notes:
- `tests/scripts/test_check_remoteapp_product_closure_audit.sh` was rerun after fixing summary ordering.
- This verifies the input-injection aggregate evidence seam only; it does not prove full RemoteApp product completion.
