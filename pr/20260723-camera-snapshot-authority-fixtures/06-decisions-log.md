# Decisions Log

- Decision: preserve the `executable_catalog()` helper name.
  Rationale: tests already use it to mark LocalRuntime-backed execution; only
  its authority construction needed convergence.
