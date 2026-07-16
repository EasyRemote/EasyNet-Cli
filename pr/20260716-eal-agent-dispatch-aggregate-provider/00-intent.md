# Intent

## Goal

Move EAL `AgentAwareDispatcher` registry loading onto the Agent aggregate read owner.

## Root Fork

EAL agent member-call dispatch is an execution path, but its production dispatcher still reads `agents.json` through `agent_registry::load_agents()` instead of consuming the aggregate-owned registered Agent projection.

## Non-goals

- Do not change EAL syntax, IR target semantics, trace shape, or public error codes.
- Do not migrate unrelated catalog/bootstrap/automation readers in this slice.
- Do not change `dispatch_to_agent` manifest resolution or daemon Invocation lowering.

## Acceptance Criteria

- `AgentAwareDispatcher::new` loads registered Agent state through `AgentAggregateRepository`.
- `src/eal/interpreter/dispatch.rs` production code contains no direct `agent_registry::load_agents()` call.
- Registry load failure keeps the existing degraded behavior: warn visibly and dispatch with an empty registry.
- Architecture convergence gate prevents reintroducing direct EAL registry reads.
- Targeted EAL dispatch tests and convergence scripts pass.
