# Decisions Log

- Decision: do not change production `AxonAbilityCatalog::new`.
  Rationale: production convenience construction should continue to fail closed
  when local Device authority is missing; only test fixtures should be
  hermetic.
- Decision: combined Device/Hub metadata tests dispatch through explicit local
  tuples.
  Rationale: ability-name lookup is intentionally insufficient when Device and
  Hub owners both register `meta.*`; the test must provide callee and subject
  authority instead of relying on registry insertion order.
