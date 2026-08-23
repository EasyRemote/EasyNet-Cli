# RemoteApp product-completion report provenance

## Problem

`remoteapp-product-completion-e2e.sh` aggregates required RemoteApp report JSONs,
but the first slices primarily checked `status`, selected coverage fields,
cross-device topology flags, and stable `script` identity. That is not enough
for a product-completion claim because an empty-shell report can still copy the
right script name, coverage keys, and status without pointing to the underlying
live artifact or exposing the product-flow/cross-device observations.

## Boundary

The top-level completion gate must validate report identity and artifact
traceability. It still does not own the per-domain evidence semantics; each
domain verifier remains the source of truth. The completion gate only verifies
that the supplied report came from the expected verifier, names an existing
evidence artifact where the domain verifier owns one, exposes the required
frontend product-flow steps for the full window+application target bundle, and
carries concrete cross-device observed pairs before trusting that verifier's
`passed` status and coverage summary. For the frontend product-flow report, the
gate validates the step artifact tree rooted beside the report instead of
trusting only the summarized `steps` array.

## Invariants

- Every required report must expose a stable `script` value.
- The completion gate must compare each report's `script` with the expected
  verifier path.
- Domain verifier reports that own a live evidence artifact must expose
  `evidence_json`, and that path must exist when the aggregate gate runs.
- The frontend product-flow report must have `target_kind=both` and include
  passed steps for Browser/Tauri lifecycle, cross-device product smoke,
  permission-subject, target-picker freshness, window/application decoded-frame,
  and window/application view-only-input coverage, so an empty or target-narrowed
  passed report cannot stand in for the user-visible lifecycle bundle.
- Each required frontend product-flow step must have a sibling step
  `result.json` with `status=passed`; Browser/Tauri, cross-device, and host
  steps must also expose their expected subreport/evidence artifacts.
- Host product-flow subreports must expose stable `script` identity, and
  decoded-frame/view-only-input host subreports must also expose the exact
  `target_kind` required by the frontend step. The completion gate must reject
  evidence where a window step is backed by application evidence, or an
  application step is backed by window evidence.
- The cross-device smoke report must include at least one observed
  caller/provider device pair where both URAs are present and distinct.
- Host lifecycle E2E reports must include the same `script` identity field as
  the other RemoteApp verifier reports.
- A wrong-script report fails closed even if it has `status=passed` and matching
  coverage.
- Child verifiers still must not set `product_complete_claim=true`.
- The top-level gate remains the only place that may set
  `product_complete_claim=true`.

## Verification

- `bash -n tools/scripts/remoteapp-product-completion-e2e.sh tests/scripts/test_remoteapp_product_completion_e2e.sh tools/scripts/check-remoteapp-product-closure-audit.sh tests/scripts/test_check_remoteapp_product_closure_audit.sh`
- `bash tools/scripts/remoteapp-product-completion-e2e.sh --self-test`
- `bash tests/scripts/test_remoteapp_product_completion_e2e.sh`
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh`
- `bash tests/scripts/test_check_remoteapp_product_closure_audit.sh`
