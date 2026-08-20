# Decisions Log

- Decision: model missing plugin permission action paths as `ActionUnavailable`.
  - Reason: a declared permission with no status/request ability is a deterministic activation blocker, not an unknown runtime condition.
