Decisions log
=============

2026-07-26
----------

- Treat missing schedule prompt as obsolete/invalid state rather than a
  heartbeat schedule feature.
- Keep the template renderer simple and bounded; this task removes only the
  missing-prompt fallback, not unknown-template-token behavior.
- Keep `ScheduleCreateSpec::new` source-compatible with the existing builder
  chain expected by the runtime-tenant gate. The unset prompt is represented as
  an invalid empty string and is rejected by `add_spec`/domain validation before
  persistence or cache insertion.
