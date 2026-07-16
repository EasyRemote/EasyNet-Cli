# API Contract

## Internal Routing Contract

`HubResolver::resolve(target_realm, target_ura)` returns:

- `HubResolution::Static` for operator-configured routes;
- `HubResolution::DirectoryFallback` only when directory fallback is enabled;
- `HubResolution::Offline` when no authorized source can route the target.

## Route Resolver Contract

`RouteResolver` passes `peer_source.allow_directory_auto_route` into
`HubResolver::new(...)` and maps each returned variant into existing route
evidence without changing public resolve output shape.
