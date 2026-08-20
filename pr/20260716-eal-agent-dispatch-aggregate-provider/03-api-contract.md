# API Contract

## Public Surface

No public CLI, EAL file syntax, IR, trace, or daemon Invocation API changes.

## Internal Contract

`AgentAwareDispatcher::new` still returns a dispatcher even when Agent state cannot be read. Internally, it logs the aggregate load failure and stores an empty registry projection.

## Error Contract

- Aggregate load failure logs an `[easynet eal] warning`.
- Missing Agent dispatch targets remain `EalError::NotFound`.
- Daemon Invocation failures remain `EalError::Unavailable` or `EalError::NotFound` according to existing classification.

## Tenant Rules

Registry keys and default-tenant fallback lookup remain unchanged in `dispatch_to_agent`.
