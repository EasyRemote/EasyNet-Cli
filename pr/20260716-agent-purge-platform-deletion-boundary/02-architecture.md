# Agent Purge Platform Deletion Architecture

## Owner Boundary

`PlatformTreeDeletion` owns the platform-specific destructive deletion decision:

- report whether identity-bound purge deletion is supported on the current target,
- reject unsupported targets before mutation,
- delete a committed quarantine through descriptor-bound traversal on supported targets.

## Layering

- `purge_agent_handler` asks the platform deletion owner to prove support before acquiring the lifecycle mutation guard.
- `finalize_committed_purge` asks the same owner to delete the committed quarantine after state-machine validation.
- Unix descriptor-bound traversal stays private to the deletion owner.
- Non-Unix targets expose no fallback deletion implementation.

## Migration Rule

Callers must not invoke standalone purge support helpers or standalone identity-bound deletion helpers. The owner object is the only lifecycle boundary for platform destructive deletion.
