# Receipt proof conformance case vocabulary convergence

## Goal

Remove compatibility-shaped vocabulary from the receipt proof conformance case. The receipt-proof-required case must describe canonical proof-boundary behavior, not an opaque compatibility exception.

## Invariants

- Receipt construction remains proof-fact mandatory.
- Conformance case metadata must not describe accepted proof projections as compatibility.
- No runtime behavior changes in this slice.
- No product-specific SDK abstraction is introduced.

## Boundary proof

- `sdk/conformance/cases/invocation-receipt-proof-required.yaml` is conformance source metadata.
- The field was not consumed by the matrix runner; it functioned as stale vocabulary rather than an executable compatibility requirement.
- The canonical-runtime-convergence v2 gate is the correct place to forbid reintroducing compatibility exception metadata for this receipt proof case.

## Refactoring plan

1. Rename the stale compatibility expectation field to proof-boundary vocabulary.
2. Add a gate check rejecting the retired field name in the receipt proof case.
3. Verify the SDK conformance matrix generation and SPEC v2 gate.

## Verification

- `python3 sdk/conformance/sdk_matrix.py --generate`
- `python3 sdk/conformance/sdk_matrix.py --self-test --tmp /tmp/easynet-sdk-matrix-self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `git diff --check`
- codegraph query for the retired field.
