Invariants

1. `provider_routes/easynet-principal-lifecycle-routes.v1.json` remains the
   only editable source for PrincipalLifecycle ability names.
2. Generated daemon route constants are crate-internal implementation facts;
   public/internal compatibility paths in `conformance.rs` and
   `principal_lifecycle.rs` are preserved as aliases.
3. No PrincipalLifecycle state transition, admission proof, persistence,
   runtime trust projection, conformance domain, call mode, or receipt semantic
   changes in this slice.
4. Go, Python, Rust CLI, and daemon generated files must carry the same
   manifest digest and pass the generator `--check`.
5. Remaining literal `principal.lifecycle.*` strings are allowed only in the
   manifest, generated files, user-facing error text, and tests that assert
   wire behavior.
