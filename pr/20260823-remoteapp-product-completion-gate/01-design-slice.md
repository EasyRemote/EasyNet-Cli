# RemoteApp product-completion evidence gate

## Problem

RemoteApp now has multiple focused E2E verifiers for capture, input, media,
multi-window tracking, lifecycle recovery, network fallback, Browser/Tauri UI,
and cross-device behavior. Those verifiers are correct as bounded owners, but
there was no single top-level gate for the product-complete claim.

Without that gate, a narrower artifact such as product-flow, host-local capture,
or cross-device synthetic media could be mistaken for full interactive remote
desktop completion.

## Boundary

`remoteapp-product-completion-e2e.sh` is an evidence aggregator only. It does not
own the evidence contracts for capture, input, media, network, frontend, or
cross-device execution. Each domain verifier remains the source of truth for its
own artifact.

The completion gate reads report JSONs from the required domain verifiers and
fails closed unless all are present and passed.

## Required reports

- frontend product-flow
- Browser/Tauri lifecycle
- cross-device smoke
- cross-platform capture
- input injection
- media adaptation
- multi-window tracking
- network fallback
- session timeout
- session cancel
- permission revoke
- session resume
- crash/restart recovery

## Invariants

- Missing reports are failure, not skipped completion.
- Child verifiers must not set `product_complete_claim=true`.
- Cross-device evidence must not be local-provider-only.
- Coverage fields that represent the user objective must be true in the relevant
  domain reports.
- Only the top-level completion gate may emit `product_complete_claim=true`, and
  only when every required report has passed.

## Verification

- `bash -n tools/scripts/remoteapp-product-completion-e2e.sh tests/scripts/test_remoteapp_product_completion_e2e.sh tools/scripts/check-remoteapp-product-closure-audit.sh tests/scripts/test_check_remoteapp_product_closure_audit.sh`
- `bash tools/scripts/remoteapp-product-completion-e2e.sh --self-test`
- `bash tests/scripts/test_remoteapp_product_completion_e2e.sh`
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh`
- `bash tests/scripts/test_check_remoteapp_product_closure_audit.sh`
