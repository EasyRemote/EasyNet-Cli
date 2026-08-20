# Decisions Log

- Storage load semantics are modeled as state, not as an implicit empty fallback.
- Missing trust-anchor storage remains non-fatal only at daemon first boot and
  CLI display boundaries.
- Reload is fail-closed because replacing live trust authority with an empty set
  is an unsafe compatibility behavior.
