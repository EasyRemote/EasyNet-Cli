# Intent

## Goal

Move invocation-facing Agent chat helper reads onto the Agent aggregate owner so hot-added discover/invoke handlers and chat peer-skill enumeration no longer read `agents.json` directly.

## Non-goals

- Do not change `<agent>.chat`, `<agent>.discover`, or `<agent>.invoke` public request/response shapes.
- Do not migrate unrelated catalog/bootstrap or EAL registry readers in this slice.
- Do not introduce compatibility fallback paths around the aggregate owner.

## Acceptance Criteria

- `agents/chat.rs` reads Agent registry state through `AgentAggregateRepository` for hot-added discover/invoke providers and peer-skill enumeration.
- The Agent aggregate exposes the projection needed by chat without leaking `local-agents.json` details into the chat module.
- Aggregate load failure remains explicit for handler construction paths and does not silently synthesize stale registry state.
- Chat peer-skill enumeration preserves its current degraded behavior of returning an empty tool hint list when durable Agent state is unavailable.
- Targeted tests and the architecture convergence gate remain green.
