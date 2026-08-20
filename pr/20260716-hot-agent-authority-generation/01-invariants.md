# Invariants

1. Hosted Agent authority generation is advanced only by
   `HotAgentAuthorityInventoryState`.
2. Hosted Agent incarnation allocation is owned by the same state object.
3. Counter overflow is a typed failure and leaves the previous state unchanged.
4. Rollback/revoke compare the enrollment incarnation before deleting an entry.
5. No registry, credential, or publication fallback is added in this slice.
