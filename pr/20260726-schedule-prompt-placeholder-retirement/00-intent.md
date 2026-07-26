Goal
====

Retire the schedule tick heartbeat placeholder path. A schedule that can fire
an Invocation must carry an explicit non-empty prompt template from creation,
through persistence, to tick execution.

Non-goals
=========

- Do not add a migration or compatibility reader for old `prompt: null`
  schedules.
- Do not change cron or misfire semantics.
- Do not introduce product-specific schedule behavior.

Acceptance criteria
===================

- `ScheduleEntry` models prompt as a required runtime fact.
- `schedule.add` requires a non-empty `prompt` field.
- Schedule persistence rejects missing, null, or blank prompt values.
- The tick runner no longer synthesizes "Scheduled fire ..." placeholder work.
