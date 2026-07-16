# Architecture

## Boundary

`HubResolver` owns the source-of-truth ordering for remote hub routing:

1. static operator peer map;
2. federated-directory observation, only when explicitly enabled;
3. offline.

`RouteResolver` owns ability-route selection and must not reconstruct hub
routing from the directory cell directly.

## Risk Closed

Without an executable guard, a future refactor could treat directory observation
as a normal fallback after a static miss. That would let peer-published endpoint
data influence outbound routing without the operator opt-in modeled in
`DaemonConfig::allow_directory_auto_route()`.
