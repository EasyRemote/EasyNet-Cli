# Invariants

- Session authority is not a product login model.
- Session authority metadata does not expose backend-specific, user-specific, or
  session-id-specific field names.
- Delegation and session authority use the same generic authority vocabulary
  where their semantics overlap.
- SDKs expose typed projections only; canonical signing and runtime enforcement
  remain daemon/runtime responsibilities.
- No URI terminology is introduced. URA remains the only address term.
- Go and Python SDK behavior remains aligned through shared fixtures,
  conformance reports, and schema validation.

