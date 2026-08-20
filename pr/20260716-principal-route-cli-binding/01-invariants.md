Invariants

1. PrincipalLifecycle daemon admission/state-machine behavior is not changed.
2. CLI command names, flags, JSON payload shape, output labels, timeout policy,
   callee URA derivation, and subject URA derivation remain unchanged.
3. Go, Python, and Rust CLI route constants are generated from the same
   manifest digest.
4. The generated Rust constants are crate-internal and are not a public Rust
   SDK or CLI API.
5. Generated route files must be deterministic under `--check` and after
   language formatters.
6. Provider route strings may still appear in manifest fixtures, user-facing
   error text, and tests that intentionally assert wire behavior.
