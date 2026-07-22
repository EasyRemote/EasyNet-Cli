# Invariants

1. Canonical SDK authority metadata keys are product-neutral.
2. Rust daemon admission, FFI tests, and language SDK fixtures use the same two key literals.
3. Public authority metadata remains typed and mutually exclusive.
4. Old `x-easynet-delegation` and `x-easynet-session-authority` keys are not accepted as a canonical fallback.
5. No compatibility alias is introduced in canonical SDK packages.
