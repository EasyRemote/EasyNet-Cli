Decisions log
=============

2026-07-26
----------

- Treat empty normalized redirect targets as invalid input, not as permission
  to substitute `/`.
- Preserve the existing public shell.run rejection class for path failures but
  keep the internal ShellGuard state machine precise enough to distinguish
  invalid targets from outside-root targets.
