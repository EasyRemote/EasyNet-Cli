# Boundary Proof

## SDK-Owned

- Typed DTO projection for authority metadata envelopes.
- Shape validation for delegated-authority and session-authority facts.
- Invocation metadata mutual exclusion.
- Transport delegation boundaries and lifecycle close semantics.

## Runtime/Provider-Owned

- Canonical authority payload construction.
- Signature materialization.
- Cryptographic verification.
- Daemon/C ABI authority provider implementation.

## Product-Owned

- Browser/user session policy.
- Product authentication and authorization UX.
- Token issuance policy outside daemon/Axon authority metadata.

## Conclusion

Node can safely expose a seam for typed authority metadata projection and
transport delegation because it preserves the canonical runtime model while
keeping authority cryptography and daemon-provider behavior below the facade.

The seam intentionally does not mirror product-facing backend/user/session
authority names. Those belong to provider or product adapters when needed. The
public Node SDK authority DTO uses issuer URA, subject URA, audience, scopes,
expiry, and signature, matching the spec's generic authority model.
