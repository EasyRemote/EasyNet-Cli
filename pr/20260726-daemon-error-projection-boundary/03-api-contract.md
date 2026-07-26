# API Contract

No public API changes.

Stable output preserved:
- `CALLER_SIGNER_UNAVAILABLE` records keep stage `caller_identity`.
- `DESCRIPTOR_OWNER_OFFLINE` records keep stage `routing`.
- Normal daemon transport errors continue to use the existing FFI status-code mapping.

Tenant and authority rules:
- The projection does not alter authorization decisions.
- It only preserves the typed reason already emitted by daemon/runtime layers.
