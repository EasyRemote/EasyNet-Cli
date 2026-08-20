# Invariants

- `[daemon].hub_endpoint` and `[daemon.federated_peers]` are different runtime
  topology facts and must not be derived from each other.
- Join authority wiring is required; if peer-hub topology is missing or
  ambiguous, join must fail before rendering the federated-peers stage as
  complete.
- No port constant may stand in for an absent topology fact.
- Endpoint resolution must have explicit source states: operator override or
  pairing TLS endpoint.
- Existing daemon-config contents must remain untouched when endpoint
  resolution fails.
