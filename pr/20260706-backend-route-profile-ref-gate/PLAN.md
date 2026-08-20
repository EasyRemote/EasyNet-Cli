# Backend Route Profile Reference Gate Plan

## Objective

Strengthen the backend route-family coverage gate so every SPEC 29.2 backend
route family maps to the exact SDK profile references expected by the daemon
SDK architecture, not only to a list of public client type names.

## Boundary

- Do not change `docs/spec/daemon-sdk-requirements-v1.md`.
- Do not inspect or modify the EasyNet backend repository directly.
- Keep backend product responsibilities limited to product auth, route shape,
  browser/API policy, and presentation.
- Keep daemon/runtime behavior and language SDK public interfaces unchanged.

## Invariants

1. Each backend route family must name the expected SDK clients.
2. Each backend route family must name the expected SDK profile references.
3. Route-family responsibility text must not claim local runtime ownership.
4. Coverage evidence must continue to point to existing shared conformance or
   static gate files.
5. The gate remains manifest-driven and deterministic.

## Implementation Steps

1. Add exact `sdk_profile_refs` expectations to the route coverage validator.
2. Add a self-test fixture that mutates profile refs and must fail.
3. Record the expectation in the shared backend route-family conformance case.
4. Update Go conformance assertions.
5. Run focused backend route coverage and Go conformance gates.

## Verification

- `tools/scripts/check-backend-route-family-coverage.sh --self-test`
- `go test ./... -run 'BackendCutover|Conformance'`
- `tools/scripts/check-sdk-parity-matrix.sh --self-test`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `git diff --check`

## Verification Result

- PASS: `tools/scripts/check-backend-route-family-coverage.sh --self-test`
- PASS: `go test ./... -run 'BackendCutover|Conformance'`
- PASS: `tools/scripts/check-sdk-parity-matrix.sh --self-test`
- PASS: `bash tools/scripts/check-sdk-scaffold.sh`
