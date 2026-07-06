# Invariants

## Semantic Invariants

- `descriptor_ref` identifies a governed AbilityDescriptor version.
- A descriptor-bound call without a descriptor version is invalid.
- An ability descriptor ref must bind an Ability URA, not an agent, device, resource, endpoint, or product route.

## Boundary Invariants

- Go delegates parsing to the existing Axon SDK helper.
- Python owns only a generic lexical seam until an Axon-backed parser is available at build time.
- Identity/profile clients still use daemon/Axon helper methods for canonical descriptor-ref construction and projection.

## Boundedness Invariants

- Validation is local, deterministic, and independent of daemon liveness.
- No transport fallback or product-specific repair path is introduced.
