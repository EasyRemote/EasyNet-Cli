# Intent

## Goal

Remove the legacy/compat `pairing_secret` ingress from the canonical `federation.join` runtime contract. `federation.join` must be driven by generic runtime facts: `membership_ura`, `realm`, `public_key_hex`, and optional product-neutral `principal_enrollment`.

## Non-goals

- Do not change token-pairing HTTP join behavior.
- Do not add product-specific EasyNet/EasyRemote SDK concepts.
- Do not introduce a compatibility alias or silent repair path.

## Acceptance criteria

- The daemon-published `federation.join` schema no longer advertises `pairing_secret`.
- SDK/client-side join argument projection can no longer serialize `pairing_secret`.
- Dispatch/admission behavior remains descriptor-bound and public API compatible for canonical join fields.
- Tests prove the removed field is not emitted or accepted as canonical contract surface.
