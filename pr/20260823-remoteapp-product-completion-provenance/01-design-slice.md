# RemoteApp product-completion report provenance

## Problem

`remoteapp-product-completion-e2e.sh` aggregates required RemoteApp report JSONs,
but the first slice primarily checked `status`, selected coverage fields, and
cross-device topology. That is not enough for a product-completion claim because
a wrong or synthetic report with compatible shape could be supplied for a domain.

## Boundary

The top-level completion gate must validate report identity. It still does not
own the per-domain evidence semantics; each domain verifier remains the source
of truth. The completion gate only verifies that the supplied report came from
the expected verifier before trusting that verifier's `passed` status and
coverage summary.

## Invariants

- Every required report must expose a stable `script` value.
- The completion gate must compare each report's `script` with the expected
  verifier path.
- Host lifecycle E2E reports must include the same `script` identity field as
  the other RemoteApp verifier reports.
- A wrong-script report fails closed even if it has `status=passed` and matching
  coverage.
- Child verifiers still must not set `product_complete_claim=true`.

## Verification

- `bash -n tools/scripts/remoteapp-product-completion-e2e.sh tests/scripts/test_remoteapp_product_completion_e2e.sh tools/scripts/check-remoteapp-product-closure-audit.sh tests/scripts/test_check_remoteapp_product_closure_audit.sh`
- `bash tools/scripts/remoteapp-product-completion-e2e.sh --self-test`
- `bash tests/scripts/test_remoteapp_product_completion_e2e.sh`
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh`
- `bash tests/scripts/test_check_remoteapp_product_closure_audit.sh`
