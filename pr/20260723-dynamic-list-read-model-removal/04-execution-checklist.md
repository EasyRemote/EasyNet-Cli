# Execution Checklist

- [x] Confirm `list_dynamic_abilities` has no production callers.
- [x] Replace its unit-test usage with the narrower `has_dynamic` diagnostic predicate.
- [x] Remove stale dynamic/static union and fall-through wording.
- [x] Add architecture convergence checks that reject the retired helper and wording.
- [x] Run focused unit test.
- [x] Run fmt and convergence gates.
- [x] Sync and query codegraph for the removed symbol.
