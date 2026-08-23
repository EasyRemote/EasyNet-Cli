# RemoteApp Cross-Device Topology Artifact Slice

## Intent

Close the evidence seam where `remoteapp-cross-device-product-smoke.sh` could
compose two child Docker E2Es but leave the parent report without a hard,
machine-readable distinction between true cross-device topology and
same-device/local-provider-only evidence.

## Boundary

This slice updates the parent smoke report contract, closure audit, and
readiness documentation. It does not change the Docker child E2Es, implement
real RemoteApp OS capture, or claim product completion.

## Product invariant

Cross-device RemoteApp evidence must not be inferred from a script name. A
report must explicitly say whether it observed:

- a caller device URA;
- a provider/callee device URA;
- distinct caller/provider device URAs;
- local-provider-only evidence.

If `local_provider_boundary_only=true`, the report may be useful diagnostics,
but it is not cross-device RemoteApp product evidence.

## Architecture decision

Keep child E2Es focused on their own Docker product paths and aggregate topology
in the parent smoke gate. The parent is the product-readiness boundary because
it decides whether the composed evidence can be used for the cross-device
RemoteApp row.

## Verification checklist

- `bash -n tools/scripts/remoteapp-cross-device-product-smoke.sh`
- `bash tools/scripts/remoteapp-cross-device-product-smoke.sh --self-test`
- `bash tests/scripts/test_remoteapp_cross_device_product_smoke.sh`
- `python3 -m json.tool docs/design/remoteapp-product-readiness-matrix.json`
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh`
- `bash tests/scripts/test_check_remoteapp_product_closure_audit.sh`
- `git diff --check`

## Non-claims

- This does not prove real OS window/application capture.
- This does not prove pointer/keyboard input injection.
- This does not prove host audio.
- This does not prove direct/STUN/TURN/EasyNet relay deployment.
- This does not prove frontend browser rendering.
