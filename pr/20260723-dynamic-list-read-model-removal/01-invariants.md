# Invariants

1. Dynamic execution rows are dispatch state, not public catalogue publication state.
2. `list_abilities` and routeability checks must not union static and dynamic execution rows.
3. A dynamic handler may be observable for collision/hot-reload diagnostics through `has_dynamic`, but not through a list projection that resembles discovery.
4. Exact authority/mode control-plane records remain the only publication authority.
5. The change must not weaken deterministic terminal invocation behavior.
