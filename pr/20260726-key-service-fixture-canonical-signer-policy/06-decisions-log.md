# Decisions Log

## 2026-07-26

- Treat the fixture mismatch as duplicated security policy ownership, not as a
  test-only string mismatch.
- Do not add a compatibility branch for `daemon-key-inventory:*`; the retired
  namespace must fail instead of being accepted.
- Expose `daemon::identity::signer_policy_ref` as the canonical daemon identity
  policy API instead of letting integration tests clone the hashing algorithm.
- Extend SPEC v2 gate coverage to include the process-local key-service fixture,
  because integration tests compile without `cfg(test)` and exercise the same
  framed custody protocol as production LocalRuntime calls.
