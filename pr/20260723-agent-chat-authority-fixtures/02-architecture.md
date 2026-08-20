# Architecture

Root abstraction problem:

The tests used `AxonAbilityCatalog::new()` for route registration, coupling chat
ability tests to ambient daemon authority state.

Refactoring:

- Add one local `agent_chat_test_catalog()` helper.
- Build it through the catalog-owned explicit Device authority fixture.
- Keep filesystem HOME isolation separate from catalog authority setup.
