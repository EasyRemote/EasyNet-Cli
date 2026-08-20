# Invariants

1. Directory read models remain daemon-owned projections; SDK helpers only normalize already-returned projection fields.
2. Public helpers must not require product repos to import Axon SDK or generated protobuf enum symbols.
3. Unknown numeric enum ordinals remain observable as decimal strings so operators can diagnose schema skew.
4. Missing or nil read-model enum values project to an empty string and do not panic.
5. String enum values pass through unchanged to preserve daemon-rendered names.
6. Non-integral numeric values remain observable as their original decimal form and are never truncated into a valid ordinal.
