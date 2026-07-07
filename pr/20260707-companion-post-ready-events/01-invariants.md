# Companion Post-Ready Event Invariants

1. Daemon Ready remains the boundary before companion ensure-running work.
2. Companion start failure after Ready must not fail daemon boot.
3. The manager returns structured reconciliation failures, not formatted CLI
   strings.
4. Operator event formatting belongs at the lifecycle boundary.
5. Status projection consumes the same companion state store used by lifecycle
   actions.
6. Observed runtime state remains re-probed; state-store memory is not process
   truth.
7. A later successful post-Ready start clears the previous start error.
8. No companion lifecycle behavior is exported as remote product ability.
