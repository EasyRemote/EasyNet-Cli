# Architecture

`DaemonRouteResolver` produces `ResolveRouteFailure`.

`ResolveRouteFailure` now carries:

- `query_name`
- typed negative reason
- typed route failure kind
- diagnostic detail

`target_gate` receives that typed failure and performs only transport mapping. It no longer owns resolver business semantics.

Layer boundary:

- routing: determines whether a route failed due to absence, owner offline, unsupported route, policy, or invalid query.
- admission: projects route failure to gRPC status while preserving diagnostics.
