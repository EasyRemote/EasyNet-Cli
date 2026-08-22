# Decisions and evidence

## Decisions

1. Treat `accepted_count=0, expected_count=5` as historical cross-device diagnosis after Service multihost projection support.
2. Require the product audit to name the Service multihost regression tests.
3. Keep cross-device product readiness partial because the latest live smoke did not pass; it failed at environment readiness with `docker info timed out after 3s`.

## Evidence to run

- `bash tools/scripts/check-remoteapp-product-closure-audit.sh`
- `bash tests/scripts/test_check_remoteapp_product_closure_audit.sh`
- `bash tests/scripts/test_remoteapp_cross_device_product_smoke.sh`
- JSON validation for `docs/design/remoteapp-product-readiness-matrix.json`
- `git diff --check`
