# Admin Pairing And Device Session Conformance Plan

## Objective

Remove the stale `pairing_and_device_session_crud: scaffold_only` marker from
the Admin + Gateway shared conformance case by proving the existing Go/Python
SDK facades expose provider-backed pairing and device-session lifecycle
operations through typed Admin DTOs.

## Invariants

- The SPEC remains unchanged.
- Pairing and device-session policy stays daemon-owned; this slice does not
  invent SDK-local trust state, token storage, or product pairing lifecycle.
- The SDK facade continues to own only typed request validation, transport
  handoff, and DTO projection.
- No lower-layer C ABI or raw daemon socket exposure is introduced into shared
  conformance tests.
- Shared fixtures use complete Admin carrier context and URA naming only.
- The change must prove Go/Python parity through the same conformance case and
  action-ownership gates.

## Implementation Steps

1. Add shared conformance fixtures for pairing preflight/create/validate,
   device-session create/list/delete, and their typed DTO projections.
2. Update `admin-gateway/carrier_status` actions and expectations from
   scaffold-only to provider-backed Admin lifecycle evidence.
3. Extend Go shared Admin transport/tests to assert fixture requests and project
   pairing/session DTOs.
4. Extend Python shared Admin transport/tests with the same request/projection
   evidence.
5. Run focused Admin conformance tests, then Go/Python full SDK tests and shared
   conformance runner gates before committing.

## Boundary Proof

The daemon owns whether pairing is required, how tokens are minted, which
device credential is accepted, and how device sessions are persisted. The SDK
may validate request shape and project daemon output into `PairingPreflight`,
`PairingToken`, `DeviceCredential`, `DeviceSession`, `DeviceSessionPage`, and
`DeviceAdminResult`. Shared conformance should prove those facade contracts
without asking the SDK to store pairing state or decide trust policy.

## Verification Plan

- `go test ./...` in `sdk/go`
- `uv run python -m unittest discover tests` in `sdk/python`
- `cargo test --bin sdk-conformance-runner`
- `cargo run --bin sdk-conformance-runner -- --language go --adapter-report
  sdk/conformance/runner/go-action-adapter-report.json --format jsonl`
- `cargo run --bin sdk-conformance-runner -- --language python --adapter-report
  sdk/conformance/runner/python-action-adapter-report.json --format jsonl`
- `git diff --check`
- `git diff -- docs/spec/daemon-sdk-requirements-v1.md`

## Verification Results

- Focused Go Admin shared conformance test: passed.
- Focused Python Admin shared conformance test: passed.
- `go test ./...` in `sdk/go`: passed.
- `uv run python -m unittest discover tests` in `sdk/python`: 456 tests
  passed.
- `cargo test --bin sdk-conformance-runner`: 9 tests passed.
- Go adapter report through `sdk-conformance-runner`: passed, including
  `admin_gateway/carrier_status`.
- Python adapter report through `sdk-conformance-runner`: passed, including
  `admin_gateway/carrier_status`.
- `git diff --check`: passed.
- `git diff -- docs/spec/daemon-sdk-requirements-v1.md`: empty.
