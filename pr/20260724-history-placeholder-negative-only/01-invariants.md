# Invariants

1. Runtime-state reads use `LocalRuntimeStateReadIssuer`.
2. The issuer derives a user-owned runtime-state resource subject from paired
   credentials.
3. The all-zero `invocation_history` placeholder is negative-test vocabulary
   only.
4. Positive history/session fixtures must use non-placeholder users.
5. Product scripts must not synthesize or fallback to the placeholder.
