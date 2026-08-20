# Invariants

## Runtime ingress

- Every product-visible invoke must enter daemon transport with explicit caller, callee, subject, ability, action, and nonce facts.
- Runtime-state reads must use a named issuer and may not fall back to daemon/device self-subject shortcuts.
- Descriptor lookup, ability catalog visibility, and route reachability must not be represented as unrelated authorities.

## Authority and identity

- All-zero principals are valid only as negative test fixtures.
- Session authority must admit the envelope subject structurally; mismatch is a hard denial.
- Missing caller signer is a provisioning/key-custody failure, not a reason to bypass canonical signing.

## Bounded behavior

- Invocation lifecycle must reach one terminal state.
- Descriptor and route failures must surface as explicit canonical errors, not as product-specific internal fallbacks.
