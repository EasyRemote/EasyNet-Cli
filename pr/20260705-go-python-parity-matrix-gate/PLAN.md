# Go/Python SDK Parity Matrix Gate Plan

## Goal

Add a machine-checkable Go/Python daemon SDK parity matrix that records one unified capability model, explicit status levels, evidence refs, and remaining gaps for each P0 daemon SDK capability.

## Boundary Proof

- The matrix is evidence over the current implementation, not a new spec and not a replacement for `docs/spec/daemon-sdk-requirements-v1.md`.
- The matrix uses one capability set for Go and Python so backend and EasyRemote do not grow separate SDK models.
- Product cutover gates are recorded separately from daemon SDK capabilities because backend cutover is Go-owned and EasyRemote cutover is Python-owned.
- Status values are constrained to `unsupported`, `seam`, `provider-backed`, and `cutover-ready`; false `cutover-ready` claims must fail validation.
- All identity/address evidence uses URA terminology only.

## Implementation Slices

1. Add `sdk/conformance/sdk-parity-matrix.json` with Go/Python status and evidence per capability.
2. Add `sdk/conformance/cases/sdk-go-python-parity-matrix.yaml` as the shared conformance case.
3. Add `tools/scripts/check-sdk-parity-matrix.sh` with self-tests for missing rows, invalid status, and false cutover claims.
4. Wire the gate into scaffold checks and Go/Python conformance tests.
5. Update SDK parity documentation and run full verification.
