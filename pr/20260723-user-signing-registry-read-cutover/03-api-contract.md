# API Contract

No public command, SDK, or output contract changes.

Internal contract:

- `identity.list_user_pubkeys` must not be invoked through
  `invoke_local_ability` from user signing identity reconciliation.
- `identity.register_pubkey` remains a mutation and must not be moved to the
  read issuer.
