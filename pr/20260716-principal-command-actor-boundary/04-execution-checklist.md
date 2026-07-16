# Execution Checklist

- [x] Replace hidden `actor_ura` fallback with `PrincipalCommandActor`.
- [x] Migrate all command construction call sites.
- [x] Update PrincipalLifecycle CLI tests for the explicit actor state.
- [x] Add an architecture convergence guard for the actor boundary.
- [x] Add a self-test case that fails on the old fallback.
- [x] Run targeted tests and the architecture convergence gate.
