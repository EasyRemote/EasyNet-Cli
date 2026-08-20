# Paired User Trust Signer Custody

## Goal

Close the session prelude authority mismatch where paired User trust bootstrap
could publish or resolve User key material while signed as the Device caller.

## Invariants

1. Device session preludes may continue to use the Device signer for Device
   session establishment.
2. Paired User trust bootstrap must load a signer whose owner is the paired User
   URA before invoking `identity.register_pubkey` or `federation.resolve_key`.
3. Device-only or federation-native credentials without a bound User remain
   `NotRequired`; malformed bound credentials remain fail-closed.
4. Tests must prove the prelude request caller is the paired User, not only that
   the request body carries a User URA argument.

## Boundary Proof

`identity.register_pubkey` and `federation.resolve_key` mutate or verify User
trust facts. Their authority subject is the User, so using the Device signer
creates an authority-subject mismatch: the envelope caller proves Device custody
while the payload asks to operate on User custody. A dedicated
`PairedUserTrustSigner` source keeps User custody explicit without broadening the
session supervisor or adding a product-specific SDK abstraction.

## Verification

- `cargo test -q paired_user_trust -- --nocapture`
- `cargo test -q dial_and_run_session -- --nocapture`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-architecture-convergence.sh`
- `cargo fmt --check`
- `git diff --check`
