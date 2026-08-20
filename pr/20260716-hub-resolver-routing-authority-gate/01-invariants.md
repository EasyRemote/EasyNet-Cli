# Invariants

- Operator-curated `federated_peers` are the authoritative remote-realm route
  source.
- Federated-directory `hub_endpoint` is an observed fact, not a default routing
  authority.
- Directory auto-route is opt-in and must be represented in code by
  `allow_directory_fallback`.
- A static peer hit must win even when directory fallback is enabled.
- A static miss with directory fallback disabled must resolve to `Offline`.
- `RouteResolver` consumes `HubResolution` variants and preserves evidence
  source labels for telemetry and route proof.
