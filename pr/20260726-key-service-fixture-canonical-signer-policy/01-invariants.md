# Invariants

- Runtime signing policy derivation has one owner:
  `daemon::identity::signer_policy_ref`.
- Test key-service fixtures must model production custody, not preserve a
  parallel security algorithm.
- A descriptor-bound local invocation must be signed only when public key,
  owner URA, and signer policy ref bind to the same projection.
- Policy mismatch remains a hard failure; no compatibility fallback is added.
