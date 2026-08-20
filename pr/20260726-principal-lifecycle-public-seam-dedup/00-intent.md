# Principal lifecycle public seam deduplication

Goal: remove the duplicated `PrincipalProvider` public seam while preserving the SPEC-required `PrincipalLifecycle` state-machine contract and the provider-backed `RuntimePrincipalProvider` implementation path.

Non-goals:
- Do not remove the canonical principal lifecycle capability.
- Do not rename or product-shape principal lifecycle routes.
- Do not add a compatibility alias for removed public symbols.

Acceptance criteria:
- Go and Python expose one public principal lifecycle seam named `PrincipalLifecycle`.
- `PrincipalClient` accepts the canonical lifecycle seam.
- `RuntimePrincipalProvider` remains the provider-backed implementation.
- Conformance inventory and parity evidence no longer list `PrincipalProvider`.
- SPEC v2 gate, focused Go/Python tests, formatting, and diff checks pass.
