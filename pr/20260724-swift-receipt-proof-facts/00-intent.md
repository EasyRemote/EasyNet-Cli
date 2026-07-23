# Intent

## Goal

Remove the Swift SDK opaque terminal receipt compatibility path and converge Swift invocation results on canonical runtime receipt proof-facts validation.

## Non-goals

- Do not add EasyNet or EasyRemote receipt concepts.
- Do not preserve `receipt_ref` as a terminal receipt substitute.
- Do not change public invocation method names unless required by canonical runtime semantics.

## Acceptance criteria

- Swift `InvocationResult` requires `terminal_receipt`.
- Swift terminal receipts are validated as canonical runtime receipts.
- Proof facts are mandatory and fail closed.
- Opaque receipt fixtures are removed from Swift tests and v2 gate coverage.
