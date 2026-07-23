# Architecture

Root abstraction problem:

The governance health dispatch smoke test used `AxonAbilityCatalog::new_with_runtime`
directly. In tests that constructor selects authority through the ambient local
test environment. That preserves runtime coverage but hides the authority root
behind a broad constructor.

Refactoring:

- Keep the test executable because it proves local dispatch wiring.
- Bind the test catalog to one explicit Device authority root.
- Reuse the canonical `new_test_runtime_for_device_authority` test seam.
- Leave production health registration untouched.
