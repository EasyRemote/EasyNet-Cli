# Intent

## Goal

Move admission target-locality checks onto the Agent aggregate read owner so `TargetGate` does not independently join `agents.json` and `local-agents.json`.

## Non-goals

- Do not change public invocation request or response shapes.
- Do not change `matches_self_target_ura` success semantics for valid hosted Agent URAs.
- Do not migrate unrelated Agent registry readers in this slice.

## Acceptance Criteria

- `TargetGate` builds its Agent target index from `AgentAggregateRepository`.
- Production `target_gate.rs` contains no direct `agent_registry::load_agents()` or `local_agents::load()` calls.
- Corrupt or unavailable aggregate reads fail closed for Agent target locality instead of accepting partial identity evidence.
- Existing local-agent and credential-plus-registry self-target tests remain green.
- Architecture convergence gate prevents reintroducing split admission reads.
