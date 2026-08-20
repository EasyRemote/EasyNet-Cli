# Decisions Log

- Decision: do not introduce a module-local helper.
  Rationale: one direct construction does not justify another abstraction; the
  catalog-owned explicit authority helper is sufficient.
