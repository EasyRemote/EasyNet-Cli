# Typed error retry-hint boundary

## Goal

Close the SPEC `error/retry_hint` conformance case across the shared SDK
capability model.

## Invariants

- Retryability is derived only from explicit `retry` hints.
- Human-readable error messages are never parsed for control flow.
- Legacy error-code aliases such as `InvalidArgument`, `DaemonDown`,
  `DAEMON_DOWN`, and `VersionIncompatible` are rejected at typed SDK decode
  boundaries.
- Go and Python expose the same retry classification semantics:
  `never` and `unknown` are not retryable; `safe` and `after_backoff` are
  retryable.
- Domain-specific uppercase extension codes may be preserved in runtime failure
  projections, but mixed-case/retired legacy aliases fail closed.

## Planned edits

- Add `sdk/conformance/cases/error-retry-hint.yaml`.
- Wire the case into Go/Python conformance tests and all action-adapter reports.
- Tighten Go runtime failure code normalization to reject legacy aliases while
  preserving canonical uppercase extension codes.
- Add the case to the scaffold manifest.

## Verification

- `go test ./...` from `sdk/go`
- `PYTHONPATH=sdk/python python3 -m pytest sdk/python/tests/test_errors.py sdk/python/tests/test_conformance.py -q`
- `bash tools/scripts/check-sdk-conformance-reports.sh`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `bash tools/scripts/check-sdk-receipt-ura-boundary.sh`
- `git diff --check`
