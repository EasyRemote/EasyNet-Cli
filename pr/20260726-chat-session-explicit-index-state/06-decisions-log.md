# Decisions Log

- Preserve public read behavior while moving production internals to explicit
  load-state modeling.
- Treat missing index as fresh-agent state only at read/write policy boundaries.
