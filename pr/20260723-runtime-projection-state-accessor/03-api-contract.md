# API Contract

## Internal Rust API

- `RuntimeSessionProjection::state(&self) -> &config::RuntimeState` borrows the
  persisted projection state.
- `RuntimeSessionProjection::into_runtime_state(self)` remains the consuming
  conversion for code that intentionally crosses into persistence shape.

## Public behavior

- CLI status fields remain unchanged.
- MCP lifecycle detail rendering remains unchanged.
- No user-visible request, response, error, or tenant rule changes.

## Tenant rules

The projection may carry realm/tenant labels, but it is not authority. Tenant
or subject admission remains outside this accessor.
