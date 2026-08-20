# Invariants

- Public schema shape must remain stable: required request field `prompt`; required response field `reply`.
- Catalog/discovery metadata must describe the canonical runtime contract, not historical migration states.
- Compatibility concerns belong in tests or migration plans, not active product-facing ability descriptions.
- No fallback or alias parser may be added as part of this change.
