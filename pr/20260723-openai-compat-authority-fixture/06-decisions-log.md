# Decisions Log

- Decision: use a metadata-only explicit authority helper.
  Rationale: the affected tests exercise registration and synchronous handler
  lookup, not LocalRuntime execution.
