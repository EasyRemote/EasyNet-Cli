# Verification

Passing commands:

```bash
bash -n tools/scripts/frontend-remoteapp-product-flow-e2e.sh tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh tools/scripts/remoteapp-product-completion-e2e.sh tools/scripts/check-remoteapp-product-closure-audit.sh tests/scripts/test_check_remoteapp_frontend_product_flow_e2e.sh tests/scripts/test_remoteapp_product_completion_e2e.sh tests/scripts/test_check_remoteapp_product_closure_audit.sh
bash tools/scripts/frontend-remoteapp-product-flow-e2e.sh --self-test
bash tools/scripts/remoteapp-product-completion-e2e.sh --self-test
bash tests/scripts/test_check_remoteapp_frontend_product_flow_e2e.sh && bash tests/scripts/test_remoteapp_product_completion_e2e.sh
bash tools/scripts/check-remoteapp-product-closure-audit.sh
targeted frontend-flow summary mutation checks: ok
git diff --check -- tools/scripts/frontend-remoteapp-product-flow-e2e.sh tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh tools/scripts/remoteapp-product-completion-e2e.sh tools/scripts/check-remoteapp-product-closure-audit.sh tests/scripts/test_check_remoteapp_frontend_product_flow_e2e.sh tests/scripts/test_remoteapp_product_completion_e2e.sh tests/scripts/test_check_remoteapp_product_closure_audit.sh pr/20260823-remoteapp-frontend-flow-summary-gate
```

These checks prove the frontend product-flow aggregate evidence contract and its fail-closed mutation protection. They do not prove RemoteApp product completion; that still requires the live report matrix accepted by `remoteapp-product-completion-e2e.sh --check`.

Note: a full `bash tests/scripts/test_check_remoteapp_product_closure_audit.sh` run was started, but the tool session returned SIGTERM before a reliable top-level exit code was captured. Its result is not counted as passing evidence for this change.
