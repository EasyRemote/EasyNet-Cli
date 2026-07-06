# Backend SDK-Only Boundary Hardening Plan

## Objective

Strengthen the Go backend cutover gate so EasyNet backend cannot keep generated
Axon protobuf packages or raw daemon socket/control endpoints after moving to
the public CLI Go SDK.

## Boundary

- Do not change `docs/spec/daemon-sdk-requirements-v1.md`.
- Do not inspect or modify the EasyNet backend repository directly.
- Keep the check as a static cutover gate over backend production Go code.
- Preserve tests and generated SDK fixtures inside EasyNet-Cli as out of scope.

## Invariants

1. Backend production code may import `easynet.run/cli/sdk/go`.
2. Backend production code must not import raw Axon SDK packages.
3. Backend production code must not import or retain generated Axon protobuf
   packages under `internal/pb/axon/v1`.
4. Backend production code must not own direct daemon gRPC/socket transport or
   raw daemon socket endpoint strings such as `control.sock` or `daemon.sock`.
5. Backend production code must not start EasyNet daemon subprocesses.

## Implementation Steps

1. Make `check-backend-sdk-only-boundary.sh` fail on generated Axon protobuf
   package files instead of only failing on imports.
2. Make direct daemon transport package ownership a first-class rejection.
3. Add raw daemon socket/control endpoint marker detection.
4. Keep marker detection source-aware by scanning Go string literals instead of
   comments or arbitrary text.
5. Extend the script self-test with generated protobuf and raw socket fixtures.
6. Record the stronger expectations in the shared backend import-ban case.
7. Update Go conformance checks and run focused verification.

## Verification

- `tools/scripts/check-backend-sdk-only-boundary.sh --self-test`
- `go test ./... -run 'BackendCutover|Conformance'`
- `tools/scripts/check-sdk-parity-matrix.sh --self-test`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `git diff --check`

## Verification Result

- PASS: `tools/scripts/check-backend-sdk-only-boundary.sh --self-test`
- PASS: `go test ./... -run 'BackendCutover|Conformance'`
- PASS: `tools/scripts/check-sdk-parity-matrix.sh --self-test`
- PASS: `bash tools/scripts/check-sdk-scaffold.sh`
- PASS: `git diff --check`
