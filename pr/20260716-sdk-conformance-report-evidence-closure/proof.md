# SDK Conformance Report Evidence Closure

## Goal

Close the SDK conformance report evidence gate after SDK implementation changes altered the source files referenced by committed adapter reports.

## Boundary

The adapter report JSON files are derived attestations. Their evidence hashes are owned by `sdk/conformance/refresh_adapter_report_evidence.py`; manual JSON editing would duplicate the source-of-truth logic and weaken the audit chain.

This slice does not change SDK runtime behavior, daemon behavior, or downstream EasyNet product smoke ownership. It only refreshes the committed evidence digests so the conformance gate can validate that each referenced test source still matches the report metadata.

## Invariants

- Adapter report schema remains version 2.
- Evidence `ref_path` values must stay repository-relative and must not escape the repository root.
- Each refreshed digest must be derived from the current referenced source file bytes.
- Public SDK behavior remains unchanged.
- Existing unrelated worktree changes remain untouched and unstaged.

## CodeGraph Evidence

- `tools/scripts/check-sdk-conformance-reports.sh` delegates committed evidence freshness to `sdk/conformance/refresh_adapter_report_evidence.py --check` before running live report validation.
- `sdk/conformance/refresh_adapter_report_evidence.py` owns report discovery, schema validation, path containment, digest computation, and write mode.
- The failing gate reported stale evidence for Go and Python adapter reports after recent SDK test source changes.

## Verification Plan

- `python3 sdk/conformance/refresh_adapter_report_evidence.py --self-test`
- `python3 sdk/conformance/refresh_adapter_report_evidence.py --write`
- `bash tools/scripts/check-sdk-conformance-reports.sh`
- `bash tools/scripts/check-sdk-canonical-public-api.sh`
- `git diff --check`

## Verification Results

- PASS: `python3 sdk/conformance/refresh_adapter_report_evidence.py --self-test`
- PASS: `python3 sdk/conformance/refresh_adapter_report_evidence.py --write`
- PASS: `SDK_CONFORMANCE_LANGUAGES=go,python bash tools/scripts/check-sdk-conformance-reports.sh`
- PASS: `bash tools/scripts/check-sdk-canonical-public-api.sh`
- PASS: `git diff --check`

Full `bash tools/scripts/check-sdk-conformance-reports.sh` advanced past evidence freshness and validated the refreshed Go/Python reports, but the all-language run remained blocked by local environment failures outside this slice:

- Rust/C ABI snapshot build failed while compiling `objc2-app-kit` because the filesystem had `No space left on device (os error 28)`.
- Swift collection failed because the local Xcode XCTest import path could not build `AppKit`/`XCUIAutomation`.

The focused Go/Python run is the acceptance gate for this slice because those are the only adapter reports whose evidence digests changed.
