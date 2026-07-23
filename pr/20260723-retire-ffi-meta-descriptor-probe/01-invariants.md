# Invariants

## Semantic

- Descriptor resolution is a catalog lookup, not an invocation.
- `meta.list_abilities` remains a normal ability, not a private resolver
  fallback.
- Provider-backed descriptor catalogs must be introduced through an explicit
  provider seam, not through hidden daemon self-calls inside FFI.

## Safety

- Resolver failures must not require caller signer custody.
- Resolver failures must not depend on target owner online status.
- Resolver failures must not expose route-negative or timeout state from an
  internal probe.

## Boundedness

- Descriptor resolution has no hidden network/daemon invocation loop.
- The resolver cannot block on an ability timeout while answering descriptor
  catalog presence.

## Recovery

- Local runtime owner descriptors continue to resolve from the local system
  catalog.
- Non-local descriptors absent from the realm catalog fail as
  `DESCRIPTOR_NOT_FOUND`.
