# Decisions Log

- Decision: migrate only read-projection call sites in this slice.
  Rationale: product action commands require separate subject lifecycle
  decisions. Moving them mechanically would replace one hidden authority model
  with another.
- Decision: enforce the migration with a source gate, not comments.
  Rationale: the failure mode is architectural drift; a deterministic gate is
  stronger evidence than local review memory.
