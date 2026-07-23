# Decisions Log

- Decision: use a metadata-only explicit Device authority helper.
  Rationale: these tests verify static media descriptor/control-plane metadata
  and do not require a LocalRuntime-backed catalog.
