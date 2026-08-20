# API Contract

- `RuntimeStatusReport::from_parts` and `from_parts_with_presence` produce an
  empty desktop companion list.
- `RuntimeStatusReport::from_parts_with_observations` accepts explicit
  companion DTO values.
- `RuntimeLifecycleService::status` uses the production companion observation
  collector.
- Runtime JSON keeps the existing `desktop_companions` field.
