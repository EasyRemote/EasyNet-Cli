# Runtime Lifecycle Execution Checklist

- [x] Load runtime lifecycle spec and project-structure authority.
- [x] Move lifecycle implementation under final project structure.
- [x] Add identity-aware start request and preflight errors.
- [x] Expand lifecycle status matrix for legacy and degraded states.
- [x] Rename retired heartbeat stop stage as legacy cleanup.
- [x] Preserve projection rollback semantics and expose commit-failed status.
- [x] Update CLI start/status/stop to consume lifecycle reports cleanly.
- [x] Run project-structure guard and narrow cargo checks.
- [x] Fix release E2E harness install path and guard it.
- [x] Commit semantically coherent changes with canonical author.
- [x] Refactor lifecycle facade into concrete discovery/projection/presence collaborators.
- [x] Move stale projection removal out of pure start classification and into the service boundary.
