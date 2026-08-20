# Architecture

## Layering

- `mission::failure_codes` owns pure classification of runtime failure reason
  strings into stable code strings and typed classes.
- Mission dispatch, bidi terminal failure, and join state choose their own
  default codes.
- The classifier upgrades only when evidence is present in current runtime
  messages.

## Boundary proof

Calling the caller-provided code a fallback implies a secondary compatibility
path. It is not. It is the default state-machine outcome for the caller's
terminal state. Renaming the API to default-code language preserves behavior
while clarifying ownership.
