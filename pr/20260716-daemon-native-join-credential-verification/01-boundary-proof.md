# Boundary Proof

## Source of Truth

`has_daemon_native_join_lineage` recognizes only tokenless credentials that
carry both `join_receipt_hash` and `hub_pubkey_b64`. That keeps backend token
verification scoped to token-paired credentials while preserving explicit
daemon-native proof material for Hub URA joins.

## Public Behavior

- Existing token-paired credentials still call `verify_credential`.
- Revoked token-paired credentials are still cleaned up.
- Hub URA join credentials bypass backend HTTP verification but keep later Hub
  session endpoint checks and daemon startup behavior unchanged.

## Non-Goals

- No change to credential file schema.
- No change to federation join receipt creation.
- No change to session endpoint reachability verification.
