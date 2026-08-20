# Decisions log

## 2026-07-26

- Treat peer-hub endpoint selection as topology ownership, not endpoint
  normalization.
- Keep the valid explicit CLI contract unchanged: `--peer-hub https://host:port`
  remains the operator override.
- Reject port-50443 inference because it produces a second authority path that
  cannot be proven from pairing facts.
- Keep standalone `auto_wire_federated_peer_from_credentials` as a no-op when
  daemon-config is absent; the production join path creates daemon-config before
  the required federated-peers stage, so missing peer-hub topology still fails
  in the real join lifecycle.
