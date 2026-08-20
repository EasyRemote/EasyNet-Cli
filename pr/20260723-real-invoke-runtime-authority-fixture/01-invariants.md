# Invariants

1. Real-invoke tests remain runtime-backed.
2. The default real-invoke runtime catalog uses an explicit combined authority
   context rooted at the fixture Device URA.
3. Realm-specific real-invoke fixtures preserve their existing explicit
   authority context.
4. Direct `AxonAbilityCatalog::new_with_runtime()` calls are removed from
   `real_invoke_tests.rs`.
5. No production constructor behavior or public ability contract changes.
