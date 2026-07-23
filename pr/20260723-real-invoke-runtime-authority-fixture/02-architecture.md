# Architecture

Root abstraction problem:

`real_invoke_tests.rs` had a shared runtime fixture, but that fixture used the
ambient `new_with_runtime` constructor. Two tests also bypassed the helper and
constructed runtime catalogs locally. That left multiple test-time authority
entry points in the broad executable harness.

Refactoring:

- Make `runtime_attached_catalog()` the single local real-invoke runtime
  fixture.
- Bind it through `new_with_runtime_and_authority_context` with an explicit
  combined authority context rooted at `authority_fixture_device_ura()`.
- Migrate ad hoc local runtime constructors to the shared helper.
- Keep realm-specific custom authority context logic unchanged.
