# Decisions and evidence

## Decisions

1. The cross-device smoke report records source revision and dirty-state.
2. The cross-device smoke report records runtime image name, image id, image creation time, and whether `--build` was requested.
3. A failed reused-image smoke remains useful environment/runtime evidence but is not authoritative current-source evidence unless provenance shows the runtime image matches the source being evaluated.

## Evidence

- `bash tools/scripts/remoteapp-cross-device-product-smoke.sh --self-test`
- `bash tests/scripts/test_remoteapp_cross_device_product_smoke.sh`
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh`
- `bash tests/scripts/test_check_remoteapp_product_closure_audit.sh`
- `python3 -m json.tool docs/design/remoteapp-product-readiness-matrix.json`
- `git diff --check`
