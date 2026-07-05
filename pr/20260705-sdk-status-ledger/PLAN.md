# SDK Status Ledger Plan

## Goal

Remove stale SDK workspace status text that still described Go and Python as
placeholder facades after the parity matrix and conformance runner evidence had
advanced them to provider-backed shipped P0 profiles.

## Boundary Proof

- SDK-owned:
  - Workspace entrypoint status.
  - References to the machine-checked parity matrix.
  - Static scaffold checks that prevent README status drift.
- Product-owned:
  - Backend and EasyRemote cutover claims.
  - Product repository import deletion and route smoke evidence.

## Invariants

1. `sdk/README.md` must not become a second capability-state source of truth.
2. Go and Python status must align with `sdk/conformance/sdk-parity-matrix.json`.
3. P1 language facades remain explicitly unsupported until implemented.
4. No README text may claim `cutover-ready` without consumer repository evidence.

## Implementation Steps

1. Replace stale placeholder/scaffold rows in `sdk/README.md`.
2. Point readers to the parity matrix and `SDK_PARITY.md`.
3. Add scaffold literals that fail if the entrypoint drifts back to placeholder
   status.
4. Verify scaffold, parity, diff hygiene, and URA-only naming on touched files.

## Verification

- `bash tools/scripts/check-sdk-scaffold.sh`
- `bash tools/scripts/check-sdk-parity-matrix.sh`
- `bash tools/scripts/check-sdk-parity-matrix.sh --self-test`
- `git diff --check`
- forbidden address-spelling scan over touched files
