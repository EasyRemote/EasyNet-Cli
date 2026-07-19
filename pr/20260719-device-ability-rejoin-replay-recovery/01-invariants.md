# Invariants

- Boot replay only registers `Installed` rows whose Ability URA owner is hosted
  by the current daemon authority context.
- Rows owned by a previous paired device are local stale state, not a runtime
  wiring bug.
- Stale rows must be removed or quarantined before replay attempts to bind live
  runtime/control-plane state.
- Fatal replay diagnostics must include enough per-row detail to identify the
  failed authority root and public ability.
- No foreign authority row may be registered into `LocalRuntime`.

