# SDK Parity Language Slice

## Goal

Make the SDK parity gate consume the same explicit language slice as focused SDK conformance runs, while preserving full seven-language validation by default.

## Concrete Use Case

`check-sdk-conformance-reports.sh` already supports `SDK_CONFORMANCE_LANGUAGES=go,python` and writes a matching focused live-result directory. The parity validator also supports `--validate-slice`, but `check-sdk-parity-matrix.sh` did not expose that state machine through the shell gate. Developers therefore had to either run all seven languages or point parity at stale all-language snapshot results, which can fail on obsolete evidence hashes unrelated to the current slice.

## Boundary

- `sdk/conformance/sdk_matrix.py` remains the canonical parity validator.
- `tools/scripts/check-sdk-parity-matrix.sh` owns shell-level environment parsing and fail-closed validation of requested language slices.
- `tools/scripts/check-sdk-cutover-readiness.sh` owns orchestration and must pass one producer/consumer language slice across the conformance-to-parity artifact boundary.

## Invariants

- Default parity validation remains full seven-language validation.
- Unknown or duplicated language slice entries fail before validation.
- Focused parity validation validates only the requested languages against the supplied live-result directory.
- Snapshot/source, run nonce, toolchain, Axon revision, evidence hash, and execution attestation checks remain owned by `sdk_matrix.py`.
- This slice does not alter SDK capability states or conformance case semantics.

## CodeGraph Evidence

- `tools/scripts/check-sdk-conformance-reports.sh` parses `SDK_CONFORMANCE_LANGUAGES` and writes only selected language results.
- `sdk/conformance/sdk_matrix.py` exposes `--validate-slice`, but `tools/scripts/check-sdk-parity-matrix.sh` previously always called `--validate`.
- A fresh focused `target/sdk-conformance-live-results` directory validates for `go,python` via `sdk_matrix.py --validate-slice go python`, while old full snapshot directories can fail on stale evidence hashes.

## Verification Plan

- `bash tools/scripts/check-sdk-parity-matrix.sh --self-test`
- `SDK_CONFORMANCE_LANGUAGES=go,python SDK_CONFORMANCE_RESULT_DIR=target/sdk-conformance-live-results bash tools/scripts/check-sdk-conformance-reports.sh`
- `EASYNET_SDK_PARITY_LANGUAGES=go,python EASYNET_SDK_PARITY_RESULTS_DIR=target/sdk-conformance-live-results EASYNET_SDK_PARITY_ALLOW_SNAPSHOT_RESULTS=1 bash tools/scripts/check-sdk-parity-matrix.sh`
- `bash tools/scripts/check-sdk-cutover-readiness.sh --self-test`
- `bash -n tools/scripts/check-sdk-parity-matrix.sh tools/scripts/check-sdk-cutover-readiness.sh`
- `git diff --check`

## Verification Results

- PASS: `bash tools/scripts/check-sdk-parity-matrix.sh --self-test`
- PASS: `bash tools/scripts/check-sdk-cutover-readiness.sh --self-test`
- PASS: `SDK_CONFORMANCE_LANGUAGES=go,python SDK_CONFORMANCE_RESULT_DIR=target/sdk-conformance-live-results bash tools/scripts/check-sdk-conformance-reports.sh`
- PASS: `EASYNET_SDK_PARITY_LANGUAGES=go,python EASYNET_SDK_PARITY_RESULTS_DIR=target/sdk-conformance-live-results EASYNET_SDK_PARITY_ALLOW_SNAPSHOT_RESULTS=1 bash tools/scripts/check-sdk-parity-matrix.sh`
- PASS: `EASYNET_SDK_PARITY_LANGUAGES=go,go EASYNET_SDK_PARITY_RESULTS_DIR=target/sdk-conformance-live-results bash tools/scripts/check-sdk-parity-matrix.sh` failed with `duplicate language slice entry: go`
- PASS: `bash tools/scripts/check-sdk-canonical-public-api.sh`
- PASS: `bash -n tools/scripts/check-sdk-parity-matrix.sh tools/scripts/check-sdk-cutover-readiness.sh`
- PASS: `git diff --check`

Full all-language parity still requires a complete current live-result directory. This slice does not claim all-language cutover; it makes focused conformance and focused parity consume the same explicit artifact boundary without weakening the default release gate.
