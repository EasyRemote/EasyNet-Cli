# Decisions log

- 2026-07-24: Selected FFI descriptor-ref derivation as the next convergence seam because codegraph showed FFI and descriptor surface both deriving the same descriptor identity facts.
- 2026-07-24: Made `AbilityDescriptor::descriptor_ref()` the only owner used by FFI catalog serialization for descriptor identity derivation.
- 2026-07-24: Kept strict descriptor hash validation in the FFI catalog entry path; the refactor removes duplicate ref construction without relaxing payload integrity checks.
- 2026-07-24: Preserved the existing `descriptor_ref is not canonical` diagnostic vocabulary so provider payload failures remain clear while the derivation owner changes.
