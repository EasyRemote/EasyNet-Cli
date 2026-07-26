# API Contract

## Public behavior

The ability names and successful response shapes remain unchanged:

- `mission.run({ "source": "...", "label": "optional" })`
- `mission.track({ "run_id": "..." })`
- `mission.cancel({ "run_id": "..." })`

## Rejection behavior

The handlers now reject:

- non-object argument payloads;
- unknown keys;
- empty required string fields;
- non-string `source`, `run_id`, and `label`.

This matches the published schemas that already set `additionalProperties: false`.
