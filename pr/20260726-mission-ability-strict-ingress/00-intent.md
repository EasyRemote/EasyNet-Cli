# Intent

## Goal

Make `mission.run`, `mission.track`, and `mission.cancel` enforce the same fail-closed argument contract that their published descriptor schemas advertise. The current schemas set `additionalProperties: false`, but the handlers read fields with `Value::get`, which silently ignores unknown legacy carrier fields and type-mismatched optional fields.

## Non-goals

- Do not change public ability names.
- Do not change EAL execution semantics.
- Do not add product-specific SDK behavior.
- Do not add compatibility aliases for older mission payloads.

## Acceptance criteria

- Mission handlers require JSON object arguments.
- Unknown fields are rejected before execution or persistence lookup.
- Optional `label` type mismatches are rejected instead of becoming the default label.
- `run_id` and `source` are read through shared strict helpers.
- SPEC v2 rejects a regression to direct `args.get(...)` compatibility parsing.
