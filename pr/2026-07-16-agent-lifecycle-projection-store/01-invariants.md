# Invariants

## Projection ownership

Within the production lifecycle handlers, paired agent projections must be
written through `AgentLifecycleProjectionStore`:

- durable agent registry projection: `agents.json`
- hosted-Agent identity projection: `local-agents.json`

The lifecycle transaction still owns the state machine. The projection store
owns how state-machine transitions reach durable projection files.

## Failure semantics

The existing compensation semantics must remain intact:

1. Mark registry/identity writes before atomic write calls so post-rename
   failures still trigger compensation.
2. Restore identity projection before registry projection during rollback.
3. Preserve purge journal stage advancement between registry and identity
   persistence.
4. Keep registrar authority commit after both local projections prove durable.

## Boundary

This slice removes duplicated write assembly inside `lifecycle.rs`; it does not
yet enforce the full Agent aggregate boundary across authoring, governance, boot
recovery, or test fixtures.
