# RemoteApp product-flow cross-device evidence gate

## Problem

The RemoteApp product objective explicitly requires cross-device smoke/regression
evidence beyond a local provider boundary. The standalone
`remoteapp-cross-device-product-smoke.sh` gate already models that lower-bound
evidence, but `frontend-remoteapp-product-flow-e2e.sh` did not require it.

That left a product-flow seam: a user could run the product-flow entrypoint and
collect frontend, daemon, and host-local evidence while still missing proof that
caller and provider are distinct device URAs.

## Boundary

`frontend-remoteapp-product-flow-e2e.sh` remains an evidence aggregator. It does
not own cross-device routing semantics and does not replace
`remoteapp-cross-device-product-smoke.sh`.

The product-flow harness now requires one of:

1. an existing cross-device smoke report supplied through
   `EASYNET_FRONTEND_REMOTEAPP_PRODUCT_E2E_CROSS_DEVICE_SMOKE_EVIDENCE_JSON`; or
2. an explicit live run using
   `EASYNET_FRONTEND_REMOTEAPP_PRODUCT_E2E_CROSS_DEVICE_SMOKE_RUN=1`.

## Invariants

- Supplied cross-device reports must have `status=passed`.
- `product_complete_claim` must remain `false`.
- `topology.requires_distinct_devices` must be `true`.
- `distinct_device_uras_observed` must be `true`.
- `local_provider_boundary_only` must be `false`.
- Coverage must include cross-device Hub routing and synthetic stream/bidi
  carrier evidence.
- This is still lower-bound cross-device evidence. It does not claim real
  macOS/Windows/Linux capture, host audio, pointer/keyboard injection, or
  NAT/TURN/EasyNet relay deployment completion.

## Verification

- `bash -n tools/scripts/frontend-remoteapp-product-flow-e2e.sh tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh tests/scripts/test_check_remoteapp_frontend_product_flow_e2e.sh`
- `bash tools/scripts/frontend-remoteapp-product-flow-e2e.sh --self-test`
- `bash tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh`
- `bash tests/scripts/test_check_remoteapp_frontend_product_flow_e2e.sh`
- `bash tools/scripts/remoteapp-cross-device-product-smoke.sh --self-test`
- `bash tests/scripts/test_remoteapp_cross_device_product_smoke.sh`
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh`
- `bash tests/scripts/test_check_remoteapp_product_closure_audit.sh`
- skipped report smoke:
  `EASYNET_FRONTEND_REMOTEAPP_PRODUCT_E2E_OUT_DIR=target/test/frontend-remoteapp-product-flow-cross-device-skip tools/scripts/frontend-remoteapp-product-flow-e2e.sh`
