# Hub Resolver Routing Authority Gate

## Goal

Pin the HubResolver routing-authority boundary in the executable architecture
gate. Remote invocation delegation must prefer operator-configured peer routes
and may consult federated-directory `hub_endpoint` observations only when the
daemon explicitly enables directory auto-route.

## Non-goals

- No change to current routing behavior.
- No new federation transport.
- No public configuration rename.
- No route resolver API change.

## Acceptance Criteria

- `HubResolver` keeps a typed `HubResolution` state with `Static`,
  `DirectoryFallback`, and `Offline`.
- Static `federated_peers` remain the first consulted routing source.
- Directory lookup remains guarded by `allow_directory_fallback`.
- `RouteResolver` delegates peer routing through `HubResolver`, not direct
  directory lookup.
- The architecture self-test fails on an unconditional directory fallback.
