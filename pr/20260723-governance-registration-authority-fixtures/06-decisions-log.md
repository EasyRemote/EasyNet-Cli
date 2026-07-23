# Decisions Log

- Decision: do not introduce per-module helper functions for single-use
  registration fixtures.
  Rationale: the catalog-owned helper already expresses the authority model;
  local wrappers would recreate fixture drift.
