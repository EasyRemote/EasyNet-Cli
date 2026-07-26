# Intent

## Goal

Retire fail-open federation wire DTO parsing. Daemon-owned federation request,
response, and directory projection DTOs must reject unknown fields instead of
silently accepting stale product carriers.

## Non-goals

- Change valid federation response shapes.
- Add migration aliases for retired directory fields.
- Move product federation concepts into the SDK.

## Acceptance criteria

- Current federation wire DTOs still round-trip.
- Unknown fields on discover, resolve, resolve-key, directory-entry, and
  directory-event DTOs fail closed.
- Existing federation wrapper strictness and canonical runtime gates remain
  green.
