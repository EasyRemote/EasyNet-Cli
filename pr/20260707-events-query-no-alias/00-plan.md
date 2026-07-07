# Events Query No-Alias Plan

## Goal

Remove legacy per-stream Events subscription request aliases and converge all
maintained SDK language surfaces on canonical event query names.

## Scope

- Go: replace `Events*SubscriptionRequest` aliases with distinct
  `DirectoryEventQuery`, `DeviceEventQuery`, `SessionEventQuery`, and
  `InvocationEventQuery` request types.
- Python: remove exported `Events*SubscriptionRequest` names and migrate tests
  to canonical event query classes.
- Node: remove `Events*SubscriptionRequest` type aliases and declare canonical
  event query interfaces in method signatures.
- Scaffold: require canonical query names and reject the old per-stream alias
  names in Go/Python SDK sources.

## Non-Goals

- No change to shared JSON fixture names.
- No product event DTOs.
- No fallback aliases for compatibility.

## Verification

- `cd sdk/go && go test ./...`
- `PYTHONPATH=sdk/python uv run pytest -q sdk/python/tests/test_events.py sdk/python/tests/test_conformance.py sdk/python/tests/test_cabi.py`
- `node --test sdk/node/test/runtime-core.test.mjs`
- `TMPDIR=/tmp bash tools/scripts/check-sdk-scaffold.sh`
- `bash tools/scripts/check-sdk-completion-audit.sh`
- `git diff --check`
