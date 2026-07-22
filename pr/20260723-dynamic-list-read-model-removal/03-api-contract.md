# API Contract

No public API changes.

Internal contract:

- `has_dynamic(ability)` answers whether a dynamic execution row exists for that ability name.
- Public catalogue listing continues to project committed control-plane rows.
- Routeability continues to require both a committed mode record and an exact execution handler for that mode.

Tenant and authority semantics are unchanged because no request envelope, receipt, signer, or route format is modified.
