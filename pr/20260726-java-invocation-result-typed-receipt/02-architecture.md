# Architecture

`RuntimeReceipt` is the canonical Java SDK receipt aggregate. `InvocationResult` is a result DTO that owns the terminal-state relationship and delegates receipt proof-fact validation to `RuntimeReceipt`.

The compatibility `Map<String, Object>` surface is a projection, not the internal authority. Product integrations should use the typed `runtimeReceipt()` accessor when reasoning about lifecycle state, proof facts, and receipt identity.

Layering:

- Canonical SDK model: `RuntimeReceipt`.
- Result DTO: `InvocationResult`.
- JSON transport adapter: `InvocationResult.fromJSON`.
- Compatibility projection: `InvocationResult.terminalReceipt()`.
