# Intent

## Goal

Converge the Java SDK invocation result surface so terminal receipts are validated and exposed through the canonical `RuntimeReceipt` aggregate, not treated only as arbitrary `Map<String, Object>` JSON.

## Non-goals

- Do not change the canonical receipt wire shape.
- Do not remove the existing `terminalReceipt()` accessor in this slice; it remains a compatibility projection over the validated receipt aggregate.
- Do not invent Java-only receipt hashing or canonical byte rules outside the Axon runtime model.

## Acceptance criteria

1. `InvocationResult` construction and JSON decoding validate terminal receipt facts through `RuntimeReceipt`.
2. `InvocationResult` exposes a typed `RuntimeReceipt` accessor for product code.
3. The existing `terminalReceipt()` map accessor remains a deeply immutable raw projection.
4. Regression coverage proves missing proof facts are rejected through both direct construction and JSON decoding.
5. SPEC v2 gate prevents `InvocationResult` from regressing to map-only receipt handling.
