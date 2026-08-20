# Invariants

1. Adapter reports remain declarative source manifests and never contain a
   test status, selector, command, or live attestation.
2. Refreshing may change only `evidence[].sha256` for evidence already
   declared in a repository-local report record.
3. Every evidence reference must resolve to an existing regular file below the
   repository root; traversal and missing files fail closed.
4. `--check` is read-only and lists every stale report binding.
5. `--write` is explicit and deterministic; a second run produces no diff.
6. The conformance runner still validates report schema, evidence scope,
   selector declaration, exact collection, and execution after a refresh.
