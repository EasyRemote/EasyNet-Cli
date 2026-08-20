# Intent

Goal: make desktop companion package update perform an explicit supervisor restart when the previous companion was running.

Non-goals:
- Do not add platform-specific update branches.
- Do not change public CLI or SDK DTO shape.
- Do not add a new supervisor trait method when stop/start already model the transition.

Acceptance criteria:
- Updating an enabled/running desktop companion performs `stop` before `start`.
- Updating a stopped companion does not call restart operations.
- Existing update action result remains schema-compatible.
