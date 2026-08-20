# CLI identity projection fail-closed convergence

## Goal

Remove CLI presentation/facade compatibility paths that silently convert paired
credential user-identity projection failures into absence. Product surfaces must
show whether a runtime user identity is bound, unbound by design, or invalid.

## Root abstraction problem

Several CLI surfaces called `Credentials::user_ura().ok()` and then omitted the
Current user row when projection failed. That makes malformed or incomplete
identity custody look like a normal paired-device state and pushes the failure
downstream into descriptor resolution or invocation admission.

## Invariants

1. Token-paired credentials with a user binding project a canonical User URA.
2. Federation-native credentials without a user binding are explicit
   `Unbound`, not hidden and not modeled as an error.
3. Token-paired credentials missing user identity remain invalid through
   existing load/save validation.
4. CLI status/auth/banner surfaces must not use `user_ura().ok()` as a
   compatibility fallback.
5. The daemon/session hot paths remain fail-closed and unchanged.

## Implementation order

1. Add a cohesive credential user-binding projection type in config.
2. Migrate status/auth/banner rendering to the explicit projection.
3. Add tests proving bound/unbound/invalid projection semantics.
4. Extend convergence gate to reject the retired CLI `.ok()` fallbacks.

## Verification

- Targeted Rust tests for config/status/auth/banner as affected.
- `cargo fmt --check`
- `git diff --check`
- `check-canonical-runtime-convergence-v2.sh`
- `check-architecture-convergence.sh`
