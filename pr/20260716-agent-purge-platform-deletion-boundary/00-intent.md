# Agent Purge Platform Deletion Boundary Intent

## Goal

Converge `agent.purge` identity-bound recursive deletion behind one explicit platform deletion owner before changing capability publication semantics.

## Expected Effect

- Architecture cleanliness: platform support, unsupported-target failure, and descriptor-bound deletion live behind one cohesive boundary.
- Effect convergence: public behavior stays compatible while the implementation has a single extension point for future non-Unix support.
- Product acceleration: future capability-state work can consume a named platform boundary instead of reverse-engineering scattered `cfg` functions.

## Non-goals

- Do not change `agent.purge` request or response shape.
- Do not change the public descriptor `capability_state`.
- Do not add compatibility fallback deletion paths.
- Do not weaken identity-bound deletion, quarantine validation, or durable purge recovery.

## Acceptance Criteria

- `agent.purge` rejects unsupported platforms before journal or registry mutation.
- Unix deletion still deletes only the quarantine whose metadata matches the committed root identity.
- The old free-function support probe is removed after callers migrate.
- The architecture convergence gate rejects regression to scattered platform purge helpers.
