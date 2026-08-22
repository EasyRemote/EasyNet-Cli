# Decisions and Evidence

## Decision

Keep the Hub API readiness gate as a separate upstream product-flow preflight.
Do not move Hub/Docker/credential recovery into the RemoteApp harness; the
harness should report the product environment state and stop before host
capture/media/input evidence when upstream readiness is false.

## Current live blocker

Latest local diagnostic artifact:

- `target/e2e/hub-api-readiness/20260823-rich-failure-check-70909/report.md`
- `target/e2e/frontend-remoteapp-product-flow/20260823-live-preflight-82429/report.md`

Observed state:

- `runtime_status=projection_present_process_missing`
- `connection_state=START_FAILED_CREDENTIAL_VERIFY`
- `connection_failure.code=START_FAILED_CREDENTIAL_VERIFY`
- `connection_failure.stage=T06_VERIFY_CREDENTIAL`
- `hub_endpoint=https://127.0.0.1:50443`
- `hub_api_endpoint=null`

This proves the current environment is blocked before RemoteApp product-flow
execution. The full product-flow report failed at
`hub-api-readiness-preflight`, so no frontend, host capture, media, or input
product evidence was executed after the failed upstream gate. It does not prove
frontend Browser/Tauri lifecycle, cross-device remote target inventory, real
app/window capture, input injection, host audio, or network fallback readiness.

## Verification scope

Expected checks:

- `bash tools/scripts/hub-api-readiness-preflight.sh --self-test`
- failed local `hub-api-readiness-preflight.sh --run` writes report JSON/MD with
  connection diagnostics
- `bash tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh`
- `bash tests/scripts/test_check_remoteapp_frontend_product_flow_e2e.sh`
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh`
- `git diff --check`
