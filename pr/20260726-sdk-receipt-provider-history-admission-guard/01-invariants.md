# Invariants

1. Receipt history reads are governance read-model operations, not public target-owned actions.
2. A history read subject must be a user-owned `runtime-state/read` resource subject.
3. Session authority must admit the read subject before receipt provider dispatch.
4. Receipt filters are ledger predicates only; they do not widen the authority subject.
5. The guard must run before descriptor resolution or runtime invocation.
