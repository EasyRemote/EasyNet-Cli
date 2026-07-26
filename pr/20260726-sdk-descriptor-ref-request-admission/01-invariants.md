# Invariants

- Descriptor resolution is a runtime provider seam, not a product fallback path.
- Provider-backed descriptor resolution must carry the caller and subject facts needed for governance-scoped resolution.
- Generic catalogue lookup can remain unsupported or unauthenticated only as a generic runtime seam; provider-backed paths cannot omit identity facts.
- Go and Python SDKs expose the same request-admission contract.
- No SDK may send all-zero principal placeholders to descriptor provider transport.
