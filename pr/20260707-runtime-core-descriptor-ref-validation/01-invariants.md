# Invariants

## Semantic Invariants

- `descriptor_ref` identifies a governed AbilityDescriptor version.
- A descriptor-bound call without a descriptor version is invalid at the Axon/daemon projection boundary.
- An ability descriptor ref must bind an Ability URA, not an agent, device, resource, endpoint, or product route.

## Boundary Invariants

- Go Runtime Core does not own descriptor-ref lexical parsing during draft construction.
- Go descriptor-ref projection and canonical construction stay behind Identity/Axon helper seams.
- Python Runtime Core does not own a lexical descriptor-ref seam.
- Python descriptor projection delegates to Identity/Addressing facade methods.
- Identity/profile clients still use daemon/Axon helper methods for canonical descriptor-ref construction and projection.

## Boundedness Invariants

- Runtime Core tuple validation is local, deterministic, and independent of daemon liveness.
- Descriptor-ref grammar validation is deterministic behind the projection facade.
- No transport fallback or product-specific repair path is introduced.
