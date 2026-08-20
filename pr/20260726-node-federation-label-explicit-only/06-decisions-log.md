# Decisions log

## 2026-07-26

- Do not show `runtime_id` as a fallback UI label. It creates a product-visible
  semantic alias for an internal stable id and makes absence of product label
  indistinguishable from a deliberate operator label.
- Keep the function non-throwing because this projection is display-only; absence
  is the correct closed state.
