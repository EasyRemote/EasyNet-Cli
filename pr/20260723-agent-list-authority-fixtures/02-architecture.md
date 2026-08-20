# Architecture

Root abstraction problem:

The tests used `AxonAbilityCatalog::new()` as a fixture even though none of the
assertions require ambient process authority. That couples product-facing agent
projection tests to daemon identity state.

Refactoring:

- Add one local `agent_list_test_catalog()` helper.
- Build it through the catalog-owned explicit Device authority fixture.
- Migrate registration/handler tests to the helper.
