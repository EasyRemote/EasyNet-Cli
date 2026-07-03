# Runtime Lifecycle API Contract

Status:

- Input: current host state.
- Output: `RuntimeStatusReport`.
- Error policy: status observation is best-effort and should classify missing
  files and closed endpoints instead of failing the CLI.

Start preflight:

- Input: `RuntimeStartRequest`.
- Output: `RuntimeStartPreflightReport`.
- Errors: stale projection removal failure, control-only daemon, identity
  mismatch, missing discovery identity for a live attach candidate.

Projection commit:

- Input: mutable daemon handle and `RuntimeState` projection.
- Output: success or typed rollback/persistence error.
- Rule: fresh-spawn rollback is allowed; attached daemon rollback is forbidden.

Stop plan:

- Input: current host state.
- Output: `RuntimeStopPlan`.
- Rule: missing projection plus daemon facts still plans daemon shutdown.

Tenant/realm rule:

- The requested realm is matched against daemon discovery identity before attach.
- Device mode also matches node id. Hub mode accepts `hub` or `both` daemon
  identity modes.
