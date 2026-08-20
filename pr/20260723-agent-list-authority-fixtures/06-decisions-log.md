# Decisions Log

- Decision: use a module-local helper for repeated `agent.list` setup.
  Rationale: the authority construction remains catalog-owned, while the module
  avoids repeated literal fixture setup across multiple handler tests.
