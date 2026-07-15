# Invariants

- `prepared_id` is required for every decoded `PreparedInvocation`.
- `request_id` may be carried as metadata, but it never identifies a prepared
  handle and cannot be used for signing, freeing, or submission ownership.
- Go and Python signing DTOs enforce the same prepared-handle rule.
- C ABI prepared-key behavior remains unchanged and continues to reject
  request-id-only references.
- No alternate address terminology is introduced.
