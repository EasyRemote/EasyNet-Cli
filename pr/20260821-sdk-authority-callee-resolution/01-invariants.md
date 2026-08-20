# Invariants

## Semantic invariants

- A User is a Principal, not an Agent.
- A Service can be a callable owner/surface.
- A Device can host, sponsor, attest, and execute; it must not be the public Invocation callee for governed abilities.
- Descriptor resolution may accept a selected execution target for catalogue lookup, but it must resolve to a callable descriptor owner before Invocation construction.
- `target_ura` / execution host and `callee_ura` / callable owner must remain observable as separate concepts.

## Authority invariants

- `SessionAuthority.callee_ura` must be Agent, Service, or Authority.
- Concrete `SessionAuthority.audience` and `DelegationProof.audience` URAs must be Agent, Service, or Authority.
- Delegation selectors such as `*` and realm-prefix audiences remain selectors, not concrete Device callees.
- Authorized session binding compares authority against the resolved Invocation callee, not the selected Device target.

## Boundedness and safety

- Descriptor provider requests must fail before transport when required caller/subject/provider fields are missing.
- Provider-backed catalogue reads must project governance read subjects consistently.
- Go and Python facades must produce the same descriptor-resolution request shape for the same runtime model inputs.
