# Intent

## Goal

Clarify and converge the ownership model for file-related abilities after the
system-ability audit.

## Non-goals

- Do not move host filesystem abilities (`fs.*`) out of the Device system
  ability family.
- Do not make the user content-addressed blob store part of the Device system
  baseline.
- Do not introduce legacy compatibility names such as `<user>.files.get`.

## Acceptance Criteria

- `fs.*` remains system/device-owned.
- `files.*` is registered as a user-owned resource surface with an explicit
  daemon-native files executor root.
- OpenAI compatibility paths dereference and invoke `files.*` through the same
  explicit authority root.
