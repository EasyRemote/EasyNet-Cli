# Boundary Proof

## Ownership

Surface readiness is a daemon Surface profile projection over `pages.health`.
The SDK owns the carrier and projection DTOs; backend rendering and product page
policy remain outside the SDK.

## Alias Removal

`SurfaceStatusRequest` and `SurfaceStatus` were compatibility aliases for the
canonical `SurfaceHealthRequest` and `SurfaceHealth` model. Removing the aliases
keeps the public method behavior but prevents language surfaces from advertising
a second input or receipt model.

## Product Boundary

No product status DTO is introduced. `SurfaceStatus` methods continue to call
the generic health projection and return `SurfaceHealth`.

## URA Discipline

This slice does not introduce address terminology or URI aliases.
