# Invariants

1. The shared case is descriptive, not a new protocol schema.
   - It references existing `SDKError.details` behavior.
   - It does not add top-level error DTO fields.

2. Both P0 languages execute behavior.
   - Python and Go must trigger a real profile validation error.
   - Python and Go must inspect `profile` and `source_ref` details.

3. Existing error semantics remain stable.
   - Code, stage, retry hint, and retryability are not redefined.
   - Existing detail keys must remain preservable.

4. URA terminology remains untouched.
   - No URI terminology is introduced.
