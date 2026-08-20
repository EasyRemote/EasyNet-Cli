# Downstream Receipt Alias Cutover Guard

## Goal

Make the downstream SDK consumer cutover gate reject Backend test/runtime adapters that still emit the retired top-level `receipt` alias in unary invocation result JSON.

## Concrete Use Case

The SDK now rejects unary invocation results that use the retired top-level `receipt` field instead of `terminal_receipt`. `check-sdk-cutover-readiness.sh` still reached expensive Backend product smoke tests before discovering that `backend/internal/sdktest/runtime_bridge.go` emitted both `receipt` and `terminal_receipt`. The boundary gate should fail at the downstream adapter source that owns the stale wire shape.

## Boundary

- EasyNet-Cli SDK owns the canonical runtime result decoder and keeps rejecting the retired alias.
- EasyNet Backend test/runtime adapters own their local fixture/result projection and must emit `admission_receipt` plus `terminal_receipt` only.
- `tools/scripts/check-downstream-sdk-consumer-cutover.sh` owns cross-repo downstream readiness checks and should catch this stale adapter before product smoke execution.

## Invariants

- No SDK decoder fallback or compatibility alias is reintroduced.
- The guard targets the Backend `sdktest` runtime bridge source rather than broad receipt payloads, because nested bidi/file receipt payloads are still valid domain data.
- The good fixture must include `admission_receipt` and `terminal_receipt`.
- The negative fixture must fail on a top-level `"receipt"` key.

## CodeGraph Evidence

- `sdk/go/runtime.go` rejects retired top-level `receipt` aliases in `NewInvocationResultFromJSON`.
- `sdk/python/easynet_sdk/runtime.py` rejects the same alias for Python parity.
- `../EasyNet/backend/internal/sdktest/runtime_bridge.go` currently emits the retired top-level `receipt` alias alongside `terminal_receipt`, causing Backend product tests to fail with `invocation result must use terminal_receipt`.
- `tools/scripts/check-downstream-sdk-consumer-cutover.sh` already validates Backend SDK adapter ownership but did not inspect this result-shape boundary.

## Verification Plan

- `bash -n tools/scripts/check-downstream-sdk-consumer-cutover.sh`
- `bash tools/scripts/check-downstream-sdk-consumer-cutover.sh --self-test`
- `bash tools/scripts/check-downstream-sdk-consumer-cutover.sh ../EasyNet/backend ../EasyRemote` should fail on `backend:runtime_bridge_result_shape:forbidden:"receipt":` until Backend removes the stale alias.
- `git diff --check`

## Verification Results

- PASS: `bash -n tools/scripts/check-downstream-sdk-consumer-cutover.sh`
- PASS: `bash tools/scripts/check-downstream-sdk-consumer-cutover.sh --self-test`
- PASS: `bash tools/scripts/check-downstream-sdk-consumer-cutover.sh ../EasyNet/backend ../EasyRemote` failed fast with `backend:runtime_bridge_result_shape:forbidden:"receipt":`
- PASS: `git diff --check -- tools/scripts/check-downstream-sdk-consumer-cutover.sh pr/20260716-downstream-receipt-alias-cutover-guard/proof.md`

The broader `check-sdk-cutover-readiness.sh` now has an earlier deterministic boundary failure for the Backend stale alias. Full cutover still requires removing that alias in the Backend working tree; the SDK must not reintroduce a decoder fallback.
