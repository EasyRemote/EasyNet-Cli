# Invariants

1. Federation directory device rows must be derived from canonical Device URAs only.
2. `node_id` must equal the parsed Device URA id, never the raw URA string.
3. Invalid directory frames must not partially mutate a `DirectoryView`.
4. Snapshot and delta stream projection must share the same canonical-device predicate.
5. The refactor must not add a compatibility alias or fallback route.
