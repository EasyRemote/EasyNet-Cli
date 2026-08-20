# Invariants

1. Runtime code must not depend on route manifest digest/profile constants.
2. Tests must continue proving route generated files match their manifests.
3. Ability-name constants remain production-visible because admission,
   dispatcher and name modules consume them.
4. Generated output must be produced by the generator, not manual drift.
5. Public behavior and wire ability names remain unchanged.
