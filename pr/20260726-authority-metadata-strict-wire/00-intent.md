# Intent

## Goal

Remove the remaining permissive JSON parsing from invocation authority metadata. Delegation and session authority metadata are security-bearing admission facts; their signed wire envelopes and canonical payloads must reject unknown fields instead of silently accepting retired or product-specific carriers.

## Non-goals

- Do not change public authority metadata keys.
- Do not change signature verification ownership.
- Do not introduce fallback parsing for historical metadata shapes.
- Do not touch user-local runtime state or product UI selection state.

## Acceptance criteria

- Delegation authority payload parsing rejects unknown fields.
- Session authority payload parsing rejects unknown fields.
- Signed authority wire envelopes reject unknown fields outside `payload` and `signature`.
- Existing canonical payloads still parse and validate.
- Focused tests, formatting, and convergence gates pass.
