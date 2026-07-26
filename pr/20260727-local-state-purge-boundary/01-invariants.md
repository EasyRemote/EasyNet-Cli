# Invariants

- New runtime state must not be repaired by accepting stale descriptor refs,
  stale signer bindings, or stale owner projections.
- Local state deletion is a product daemon lifecycle operation owned by
  EasyNet-Cli, not by the canonical SDK.
- The purge boundary must be explicit and irreversible.
- A malformed runtime projection must still block non-forced reset before any
  destructive action.
- Reset must not invent caller signer, subject, nonce, or causal context facts.
