# Feature Discovery Conformance Plan

## Goal

Promote SDK-root feature discovery from ABI-version piggyback coverage to a
dedicated shared conformance case.

## Scope

- Add a language-neutral `runtime_core/feature_discovery` conformance case.
- Assert the canonical feature-discovery fixture in Go and Python shared
  conformance tests.
- Add action-adapter report records for all current SDK language reports.
- Register the case in scaffold validation.

## Non-Goals

- No product-specific feature flags.
- No EasyNet or EasyRemote product lifecycle naming.
- No compatibility aliases for older feature names.
- No change to runtime provider state claims.

## Verification

- `bash tools/scripts/check-sdk-conformance-reports.sh`
- `cd sdk/go && go test ./...`
- `PYTHONPATH=sdk/python uv run pytest -q sdk/python/tests/test_conformance.py sdk/python/tests/test_client.py`
- `TMPDIR=/tmp bash tools/scripts/check-sdk-scaffold.sh`
- `bash tools/scripts/check-sdk-completion-audit.sh`
- `git diff --check`
