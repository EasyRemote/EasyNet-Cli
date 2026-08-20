# Invariants

1. Receipt proof facts are mandatory before any terminal result can be observed.
2. `terminal_state`, `ok`, and receipt lifecycle state must agree.
3. Public compatibility may expose a raw projection, but the SDK-owned semantic object is `RuntimeReceipt`.
4. The Java SDK must not define a Java-specific receipt canonicalization algorithm.
5. Caller-provided maps must be defensively copied before validation and exposed as immutable projections.
