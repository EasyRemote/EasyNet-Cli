# Invariants

- A running companion update must not spawn a second process without first requesting stop.
- Restart is a manager-owned lifecycle transition composed from supervisor `stop` and `start`.
- Desired state must be preserved across update.
- Failure remains transactional through existing installer rollback paths.
