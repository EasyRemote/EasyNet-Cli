# Invariants

1. No protocol or schema change.
   - `SDKError.Details` carries profile metadata.
   - No daemon DTO, C ABI, or Axon shape changes.

2. Profile-local errors are traceable.
   - Profile validation and client lifecycle errors include profile refs.
   - Profile transport wrappers add refs to wrapped SDK errors without
     overwriting existing detail keys.

3. Existing behavior is preserved.
   - Error `Code`, `Stage`, `Retry`, `Retryable`, and message behavior stay
     compatible.
   - Existing details such as `reason` remain intact.

4. Runtime Core remains profile-free.
   - Generic runtime helpers stay generic.
   - Profile files call profile-aware wrappers.
