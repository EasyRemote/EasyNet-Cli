# Swift invocation receipt alias rejection

## Goal

Make Swift `InvocationResult` fail closed on retired `receipt` aliases and
missing or malformed `terminal_receipt` facts.

## Root abstraction problem

The Swift runtime facade accepted invocation result JSON that carried only the
retired `receipt` alias by silently projecting an empty `terminalReceipt` map.
The same helper also downgraded missing or malformed `terminal_receipt` values
to an empty map. That preserves a compatibility shadow path and hides provider
receipt/proof defects from products.

## Invariants

1. `InvocationResult.fromJSON` rejects the retired top-level `receipt` alias.
2. `terminal_receipt` is required for every invocation result projection.
3. `terminal_receipt` must be an object.
4. Public Swift result field names remain stable.
5. SPEC v2 rejects reintroducing optional terminal receipt projection in Swift.

## Boundary proof

- Runtime providers own terminal receipt construction and proof facts.
- The Swift SDK is a typed projection layer; it must not invent an empty
  receipt when provider facts are absent.
- Product code can still access `terminalReceipt` through the existing public
  property, but malformed provider output now fails before product state is
  updated.

## Verification plan

- Swift runtime core tests.
- Canonical runtime convergence v2 gate.
- Canonical public API gate if inventory changes.
- Repository formatting checks.
- Codegraph sync/status.

## Decisions

- Treat `terminal_receipt` as a required runtime fact in Swift, matching the
  fail-closed Java/Node direction.
- Reject the retired top-level `receipt` alias before constructing
  `InvocationResult`.
- Use a receipt-specific decoder helper instead of a generic optional string
  map, so the SDK source and gate both encode receipt ownership explicitly.

## Delta

- `InvocationResult.fromJSON` now rejects `receipt`.
- Missing `terminal_receipt` now raises validation instead of returning an empty
  map.
- Non-object `terminal_receipt` now raises validation instead of returning an
  empty map.
- Swift runtime tests now cover canonical receipt, retired alias, missing
  receipt, and malformed receipt.
- SPEC v2 now includes Swift receipt projection checks alongside Java and Node.

## Results

- `swift test --package-path sdk/swift --filter RuntimeCoreSeamTests.testInvocationResultUsesTerminalReceipt`
  passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh` passed.
- `swift test --package-path sdk/swift` passed: 20 XCTest tests.
- `cargo fmt --check` passed.
- `tools/scripts/check-architecture-convergence.sh` passed.
- `tools/scripts/check-sdk-canonical-public-api.sh` passed.
- `codegraph sync . && codegraph status .` completed with an up-to-date index.
