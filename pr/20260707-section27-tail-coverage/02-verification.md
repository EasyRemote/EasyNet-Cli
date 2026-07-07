# Verification

All commands were run from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`
on 2026-07-07.

## Targeted Gates

- `bash tools/scripts/check-sdk-section27-coverage.sh --self-test`
  - Result: pass.
- `bash tools/scripts/check-sdk-scaffold.sh`
  - Result: pass.
- `bash tools/scripts/check-sdk-section27-coverage.sh`
  - Result: pass.
- `cd sdk/go && go test . -run 'TestGoMEMCExecutesSharedSemanticAlignmentConformanceCase|TestPreparedInvocationRejectsMissingCanonicalBytes' -count=1`
  - Result: pass.
- `PYTHONPATH=sdk/python python3 -m pytest -q sdk/python/tests/test_conformance.py::SharedConformanceFixtureTests::test_python_memc_executes_shared_semantic_alignment_conformance_case sdk/python/tests/test_signing.py::SigningTests::test_prepared_invocation_rejects_missing_canonical_bytes`
  - Result: pass.

## Live Smoke Gates

- `bash tools/scripts/python-sdk-live-smoke.sh`
  - Result: pass.
  - Covered unary invoke, typed terminal failure, stream frame delivery, and bidi
    file transfer through the Python SDK facade.
- `bash tools/scripts/go-sdk-live-smoke.sh`
  - Result: pass.
  - Covered unary invoke, typed terminal failure, stream frame delivery, and bidi
    file transfer through the Go SDK facade.

## Aggregate Gate

- `bash tools/scripts/check-sdk-completion-audit.sh`
  - Result: pass.
  - Covered scaffold, parity matrix, conformance reports, Section 27 coverage,
    FFI ABI v4 header, package metadata, URA naming, receipt URA boundary,
    daemon latest input boundary, daemon Invocation migration, EasyRemote SDK
    boundary, backend route-family coverage, backend SDK-only boundary,
    EasyRemote product tests, backend product tests, Python SDK live smoke, and
    Go SDK live smoke.

## Diff Hygiene

- `git diff --check`
  - Result: pass.
