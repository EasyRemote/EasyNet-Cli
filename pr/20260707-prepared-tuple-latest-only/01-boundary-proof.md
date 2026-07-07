# Boundary Proof

## Root Abstraction

`tuple` is an `InvocationDraft` DTO. `signing_material` is daemon/Axon-owned canonical signing material. Moving signing-material fields out of `tuple` is not an SDK responsibility; accepting and rewriting that shape creates a compatibility layer outside the SPEC.

## Convergence Decision

Both SDKs now route prepared `tuple` decoding through the same canonical invocation decoder:

- Go: `requiredDraft` calls `NewInvocationDraftFromJSON(raw)`.
- Python: `PreparedInvocation.from_json` calls `InvocationDraft.from_json(...)` on the raw `tuple` object.

This gives both SDKs the same latest-only behavior and the same unknown-field rejection semantics.

## Compatibility Review

No public SDK API was removed. The removed behavior was an internal fallback that silently accepted obsolete prepared tuple shapes. The current C ABI prepared result shape already emits a clean `tuple`, so no legacy alias is required.
