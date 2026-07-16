# Agent Purge Platform Deletion Invariants

## Semantic Invariants

- `agent.purge` is destructive and requires Manage authority.
- Unsupported platforms must fail before durable mutation or live registrar mutation.
- Final deletion is allowed only after the root has been committed into quarantine and the durable registry/index removal has committed.
- The quarantined path must still match the journaled `AgentRootIdentity` immediately before recursive deletion.

## Safety Invariants

- Recursive deletion must not follow symlinks.
- Directory entries must be rechecked before unlink.
- Replacement paths must survive if the claimed directory identity changes.
- Recovery must preserve residual quarantine paths when deletion cannot prove identity.

## Boundedness Invariants

- Purge retains one local transaction guard.
- Publication retry remains in the durable outbox and is not coupled to directory deletion.
- Unsupported-platform rejection is deterministic and does not depend on runtime registry state.
