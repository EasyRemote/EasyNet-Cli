# Invariants

- `RuntimeStatusReport::to_json` is a pure renderer over report state.
- Desktop companion status values in runtime JSON are shared DTO projection
  values, not locally redefined fields.
- Runtime lifecycle classification remains independent from GUI/session
  availability.
- Existing `RuntimeStatusReport::from_parts` callers remain side-effect-free.
- Production `RuntimeLifecycleService::status` still includes current desktop
  companion observations.
