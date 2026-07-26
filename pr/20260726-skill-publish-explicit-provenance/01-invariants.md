# Invariants

1. Provenance is explicit state, not an inferred product lifecycle.
2. `InstallRecord.source` remains the single on-disk provenance owner for
   installed skills.
3. Public API compatibility is preserved: callers may omit `mission_run_id`.
4. Omitted `mission_run_id` means direct runtime publication, not implicit
   Mission authorship.
5. Audit data must never claim a curator or Mission run that was not supplied by
   the caller.
6. The skill resource path and content hash semantics are unchanged.
