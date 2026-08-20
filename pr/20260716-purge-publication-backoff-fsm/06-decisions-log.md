# Decisions Log

- 2026-07-16: Selected the Agent purge publication FSM because current source
  exploration shows a real retry-state repair already isolated in
  `agent_lifecycle.rs`, and the untracked spec matches the durable FSM boundary.
- 2026-07-16: Kept this slice narrower than full purge implementation. The
  committed behavior change is only the scheduled-drain backoff eligibility
  correction plus normative FSM documentation.
