Execution checklist
===================

- [x] Change `ScheduleEntry.prompt` to a required string.
- [x] Require prompt in `ScheduleCreateSpec` and `schedule.add`.
- [x] Make store parsing reject missing/null/blank prompt rows.
- [x] Remove tick runner synthesized placeholder prompt branch.
- [x] Update tests and add missing/blank prompt negative coverage.
- [x] Run targeted Rust tests and convergence gates.
