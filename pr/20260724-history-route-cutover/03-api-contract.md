# API Contract

## Request Rules

- `meta.list_abilities` and similar target-owned system abilities may use target-owned subject policy.
- `invocation.history.list`, `invocation.history.get`, `invocation.history.path`, and `invocation.trace.get` must use the receipt history path.
- Callers that try to invoke receipt history through target-owned remote system dispatch receive a deterministic local error.

## Error Rules

- The facade error must name the invalid boundary: receipt history is not a target-owned remote system ability.
- The error must not expose keyring internals or daemon route/probe internals.
- The error must not instruct callers to use a legacy fallback path.

## Tenant Rules

History queries remain scoped by explicit URA filters and authority metadata. No tenant or owner scope is inferred from the target Device URA.
