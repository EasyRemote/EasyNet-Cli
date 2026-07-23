# Execution Checklist

- [x] Inspect current worktree state.
- [x] Identify Swift opaque receipt seam.
- [x] Add Swift `RuntimeReceipt`.
- [x] Route `InvocationResult` through canonical receipt validation.
- [x] Replace opaque receipt tests with canonical receipt fixtures.
- [x] Update v2 gate to forbid Swift opaque receipt fallback.
- [x] Run Swift tests and architecture gates.
- [x] Commit stable changes with required author.

## Execution notes

- Added Swift `RuntimeReceipt` and internal `RuntimeReceiptProofFacts` validation.
- Bound `InvocationResult.terminal_receipt` to canonical receipt validation before exposing the public string projection.
- Replaced `receipt_ref` test fixtures and transport responses with canonical receipt objects.
- Strengthened the v2 gate to reject Swift opaque receipt fixtures and require proof-fact validation semantics.
