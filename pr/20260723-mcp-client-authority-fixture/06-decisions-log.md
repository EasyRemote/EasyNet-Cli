# Decisions Log

- Decision: use a metadata-only explicit Device authority helper.
  Rationale: the affected tests exercise registration and handler dispatch
  shape, not LocalRuntime execution.
