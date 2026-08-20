# Decisions Log

## 2026-07-24

- Treat missing top-level prepared `descriptor_ref` as provider/runtime
  non-conformance, not as an SDK recoverable condition.
- Apply the rule to Go, Python, Node, and Java in one change to avoid
  language-specific architectural divergence.
- Migrate old-shape Go/Python session provider fixtures to emit explicit
  prepared descriptor facts instead of weakening SDK validation.
