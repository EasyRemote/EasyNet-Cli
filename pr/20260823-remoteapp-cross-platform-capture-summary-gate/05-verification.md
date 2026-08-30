# Verification

Executed commands:

```bash
bash -n tools/scripts/remoteapp-cross-platform-capture-e2e.sh tools/scripts/remoteapp-product-completion-e2e.sh tools/scripts/check-remoteapp-product-closure-audit.sh tests/scripts/test_remoteapp_cross_platform_capture_e2e.sh tests/scripts/test_remoteapp_product_completion_e2e.sh tests/scripts/test_check_remoteapp_product_closure_audit.sh
bash tools/scripts/remoteapp-cross-platform-capture-e2e.sh --self-test
bash tools/scripts/remoteapp-product-completion-e2e.sh --self-test
bash tests/scripts/test_remoteapp_cross_platform_capture_e2e.sh
bash tests/scripts/test_remoteapp_product_completion_e2e.sh
bash tools/scripts/check-remoteapp-product-closure-audit.sh
bash tests/scripts/test_check_remoteapp_product_closure_audit.sh
git diff --check -- ...
```

Results:

- PASS: syntax check.
- PASS: cross-platform capture verifier self-test.
- PASS: product-completion aggregate self-test.
- PASS: cross-platform capture wrapper test.
- PASS: product-completion wrapper test.
- PASS: product closure audit.
- PASS: product closure audit wrapper.
- PASS: diff whitespace check.
