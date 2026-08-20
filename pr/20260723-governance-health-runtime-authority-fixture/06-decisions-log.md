# Decisions Log

- Decision: preserve executable LocalRuntime coverage and replace only the
  ambient authority constructor.
  Rationale: `observe.health` validates startup dispatch wiring; weakening the
  test to handler-only coverage would remove useful runtime evidence.
