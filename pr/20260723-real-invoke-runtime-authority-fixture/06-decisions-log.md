# Decisions Log

- Decision: keep realm-specific authority context construction unchanged.
  Rationale: those tests declare hosted Agent roots and already use the explicit
  authority-context constructor; collapsing them into the Device-only helper
  would remove required fixture semantics.
- Decision: use an explicit combined authority context for the default
  real-invoke fixture rather than the Device-only test helper.
  Rationale: the real-invoke harness validates mixed local runtime registration
  surfaces; Device-only filtering changes the fixture semantics and hides
  product-path failures behind an artificial authority set.
