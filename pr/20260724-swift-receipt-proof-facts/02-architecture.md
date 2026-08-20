# Architecture

## Owner boundary

Swift `InvocationResult` owns result topology. Swift `RuntimeReceipt` owns receipt summary and proof-facts validation.

## Cross-language parity

Swift should converge toward:

- Go/Python `RuntimeReceipt` proof-facts validation.
- Java `RuntimeReceiptProofFacts`.
- Node `validateRuntimeReceiptProofFacts`.

## Migration path

Replace opaque map passthrough with canonical receipt validation while preserving the public `terminalReceipt` projection as a dictionary.
