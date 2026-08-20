# Invariants

- `admission.explain` reads admission action from the persisted invocation
  ledger record.
- The ledger action must agree with the signed descriptor reference action.
- `safe_read` must agree with the decoded action; mismatches fail closed.
- Voice RPC and stream abilities project their own bound actions without a
  voice-specific branch in explain projection.
- Observer redaction behavior remains unchanged.
