# Execution Checklist

- [x] Inspect current worktree state.
- [x] Use codegraph/rg to identify one active legacy seam.
- [x] Confirm root abstraction and owning layer.
- [x] Refactor lower-level owner first.
- [x] Migrate callers.
- [x] Remove obsolete compatibility code.
- [x] Run targeted tests and gates.
- [x] Commit stable changes with required author.

## Execution notes

- Selected the Java runtime receipt proof-facts seam because Go/Python already validate canonical proof facts while Java only validated field shape.
- Added `RuntimeReceiptProofFacts` as the Java SDK owner for receipt proof-fact semantics.
- Migrated `RuntimeReceipt` to summary/lifecycle ownership plus delegation to the proof-fact owner.
- Updated the v2 gate so it verifies the new Java proof-fact owner instead of requiring procedural accumulation inside `RuntimeReceipt`.
