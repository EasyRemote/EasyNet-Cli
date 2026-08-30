# SDK Runtime Host Vocabulary

## Problem

The generic Go/Python SDK runtime-host boundary still fails because
`sdk/go/runtime.go` describes authority-binding JSON as a daemon projection.
That wording leaks EasyNet product daemon vocabulary into the SDK runtime model.

## Invariants

- Generic SDK code must describe runtime-host/provider concepts, not EasyNet
  daemon lifecycle or product policy.
- Public behavior and JSON shape remain unchanged.
- The fix is vocabulary and ownership alignment only; authority binding parsing
  remains in the SDK runtime abstraction.

## Implementation

- Replace daemon-owned wording in `sdk/go/runtime.go` with runtime-host
  projection wording.
- Re-run `check-sdk-runtime-host-vocabulary-boundary.sh` and the aggregate
  canonical runtime convergence gate.

## Verification

- `bash tools/scripts/check-sdk-runtime-host-vocabulary-boundary.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
