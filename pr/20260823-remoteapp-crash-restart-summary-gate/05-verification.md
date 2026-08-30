# Verification

Executed commands:

```bash
bash -n tools/scripts/remoteapp-product-completion-e2e.sh tools/scripts/remoteapp-crash-restart-recovery-e2e.sh tests/scripts/test_remoteapp_product_completion_e2e.sh tests/scripts/test_remoteapp_crash_restart_recovery_e2e.sh
bash tools/scripts/remoteapp-crash-restart-recovery-e2e.sh --self-test
bash tests/scripts/test_remoteapp_crash_restart_recovery_e2e.sh
bash tools/scripts/remoteapp-product-completion-e2e.sh --self-test
bash tests/scripts/test_remoteapp_product_completion_e2e.sh
bash tools/scripts/check-remoteapp-product-closure-audit.sh
bash tests/scripts/test_check_remoteapp_product_closure_audit.sh
git diff --check -- tools/scripts/remoteapp-product-completion-e2e.sh tools/scripts/remoteapp-crash-restart-recovery-e2e.sh tests/scripts/test_remoteapp_product_completion_e2e.sh tests/scripts/test_remoteapp_crash_restart_recovery_e2e.sh pr/20260823-remoteapp-crash-restart-summary-gate
```

Results:

- PASS: syntax check.
- PASS: crash/restart recovery verifier self-test.
- PASS: crash/restart recovery wrapper test.
- PASS: product-completion aggregate self-test.
- PASS: product-completion wrapper test.
- PASS: product closure audit.
- PASS: product closure audit wrapper.
- PASS: diff whitespace check.
