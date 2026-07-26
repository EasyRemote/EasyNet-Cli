# Architecture

Canonical shape:
- `PrincipalLifecycle`: state-machine seam required by `principal/lifecycle_seam`.
- `PrincipalClient`: cohesive facade delegating to `PrincipalLifecycle`.
- `RuntimePrincipalProvider`: provider-backed adapter lowering lifecycle transitions through generic runtime abilities.

Retired shape:
- `PrincipalProvider`: duplicate interface/alias that split the public model without adding a capability state.

Boundary:
- SDK remains product-neutral and provider-backed.
- Runtime route manifests remain the single source for transition lowering.
