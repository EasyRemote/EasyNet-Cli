# API contract

No public Ability or input frame schema changes.

- Data channel: `easynet.remote_desktop.input.v1`.
- Accepted pointer/key actions remain `move/down/up/wheel` and `down/up`.
- Applied/rejected events retain client sequence, host timing, target geometry,
  focus epoch, and fresh target-guard proof.
- Capability metadata adds executable backend/reason detail without claiming
  product readiness.
