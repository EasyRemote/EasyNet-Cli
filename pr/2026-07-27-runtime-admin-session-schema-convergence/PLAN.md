# Runtime admin session schema convergence

## Goal

Remove product-shaped runtime-admin session wire-field branches from Go and Python SDK production code.

## Problem

The canonical SDK runtime-admin session projection already exposes generic runtime concepts:

- `runtime_host_ura`
- `control_authority_ura`
- `session_id`
- `state`

However, Go and Python production parsers still carry explicit retired-field branches for `device_ura` and `authority_ura`. That keeps product-era field names in the canonical SDK implementation and makes the parser a compatibility artifact instead of a schema boundary.

## Architecture decision

Runtime-admin session rows are exact canonical schema rows. Product-shaped aliases are not a separate migration state; they are unknown fields.

The SDKs should:

- expose only generic `RuntimeSession` fields;
- parse session rows through one schema guard;
- reject unknown fields with a generic canonical-schema error;
- keep Go/Python behavior aligned.

## Implementation steps

1. Replace Go retired-field checks with a row schema decoder using `DisallowUnknownFields`.
2. Replace Python retired-field checks with a shared allowed-field schema guard.
3. Reclassify tests from "retired field" to "non-canonical field" rejection.
4. Update SDK product-neutrality and canonical v2 gates to require schema guards and ban retired-field messages.
5. Run focused Go/Python tests and convergence gates.

## Verification

- `go test ./...` in `sdk/go` or focused runtime-admin test.
- `uv run python -m pytest -q tests/test_runtime_admin.py` in `sdk/python`.
- `bash tools/scripts/check-sdk-product-neutrality.sh`.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`.
- `bash tools/scripts/check-architecture-convergence.sh`.
