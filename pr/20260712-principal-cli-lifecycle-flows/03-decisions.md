# Decisions

- Expose all remaining provider-backed transitions under the existing
  product-neutral `easynet principal` group instead of expanding
  Backend-oriented `easynet auth`.
- Use daemon key-service public projections for locally generated add, rotate
  and recovery keys. CLI accepts explicit public projections for remote-device
  workflows but never accepts private-key material.
- Keep proof verification, idempotency, state validation and replay protection
  inside the daemon PrincipalLifecycle provider.
- Keep standalone-Hub cutover open until no-Backend URA join and multi-user
  lifecycle E2E gates prove the commands against a running Hub.
- Add negative CLI E2E assertions for repeated recovery proof use and recovery
  after deletion. These are operator-facing UX checks over the same daemon
  state machine, not new CLI-side authorization logic.
