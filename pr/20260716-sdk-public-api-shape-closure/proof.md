# SDK Public API Shape Closure

## Intent

Close the canonical SDK public API inventory after the Python SDK runtime
failure-code slice changed exported public shapes. The inventory is the
authoritative contract for SDK conformance; leaving it stale makes the
canonical gate fail even though the implementation and focused tests pass.

## CodeGraph Evidence

- `tools/scripts/check-sdk-canonical-public-api.sh` runs
  `sdk/conformance/sdk_concepts.py --validate-actual` and compares generated
  output from `sdk/conformance/rebuild_public_api_model.py` and
  `sdk/conformance/sdk_matrix.py`.
- Current failure:
  `public_shape_mismatch:python:...changed=BidiFrame,BidiFrame.from_json,InvocationResult,InvocationResult.from_json,SDKError,SDKError.code,StreamEvent,StreamEvent.from_json,canonical_failure_code,error_class_for_code`.
- The shape drift is rooted in the Python public `SDKError.code` type and
  `canonical_failure_code` return type introduced by the previous SDK parity
  commit. `BidiFrame`, `StreamEvent`, and `InvocationResult` are listed because
  their exported `from_json` shapes resolve through the changed public error
  functions.
- `sdk/conformance/rebuild_public_api_model.py --write` is the repo-owned
  regeneration path for `sdk/conformance/canonical-public-api.json` and
  `sdk/conformance/sdk-parity-matrix.json`.

## Boundary Decision

Do not hand-edit individual shape hashes. Regenerate the canonical manifest and
matrix from the current public AST inventories so the conformance contract
matches the public SDK surface exactly.

## Verification Plan

- `bash tools/scripts/check-sdk-canonical-public-api.sh`
- `bash tools/scripts/check-sdk-canonical-public-api.sh --self-test`
- `PYTHONPATH=sdk/python:sdk/python/tests python -m pytest sdk/python/tests/test_conformance_gates.py -q`
- `go test ./... -run TestConformanceSDKProductNeutrality|TestConformanceSevenLanguageCapabilityMatrix`
- `git diff --check`

## Verification Results

- PASS: `bash tools/scripts/check-sdk-canonical-public-api.sh`
- PASS: `bash tools/scripts/check-sdk-canonical-public-api.sh --self-test`
- PASS: `bash tools/scripts/check-sdk-parity-matrix.sh --self-test`
- PASS: `PYTHONPATH=sdk/python:sdk/python/tests python -m pytest sdk/python/tests/test_conformance_gates.py -q`
- PASS: `go test ./... -run 'TestConformanceSDKProductNeutrality|TestConformanceSevenLanguageCapabilityMatrix'`
- PASS: `git diff --check`

## Residual Evidence

- `bash tools/scripts/check-sdk-cutover-readiness.sh` is not closed: product
  smokes in the downstream EasyNet backend still use retired result `receipt`
  aliases and now fail against the SDK's strict `terminal_receipt` boundary.
  That is a downstream consumer cutover slice, not part of this generated
  inventory repair.
