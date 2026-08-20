# Invariants

## Semantic

- Descriptor resolution is not invocation dispatch.
- Descriptor resolution may read local runtime catalog state and provider-backed
  realm catalog state.
- Descriptor resolution must not create, sign, or submit an invocation.
- Missing descriptor data is a hard catalog miss.

## Security and Tenant Boundary

- A descriptor resolver must not require caller key custody, because it must not
  act as a hidden caller.
- A descriptor resolver must not substitute local runtime owner identity for a
  missing public tuple field.
- Remote owner availability must not be inferred by attempting a side-effecting
  route probe from the FFI resolver.

## Boundedness

- Resolver work is bounded to local catalog construction plus one daemon-local
  realm catalog read.
- Resolver failure must be deterministic and queryable as a typed ABI error.
- No resolver branch may wait on remote network presence.

## Recovery

- Resolver failure leaves no signer state, route state, receipt state, stream
  state, or retry state behind.
