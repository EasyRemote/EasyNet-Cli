# Intent

## Goal

Move the ability-health scanner's paired Agent registry and hosted LLM identity read onto the Agent aggregate owner.

## Non-goals

- Do not change health status vocabulary or catalog metadata keys.
- Do not make health records an invocation admission gate.
- Do not migrate unrelated single-file Agent readers in this slice.

## Acceptance Criteria

- `ability/health.rs` loads Agent scan input through `AgentAggregateRepository`.
- Production `ability/health.rs` does not directly call `agent_registry::load_agents()` or `local_agents::load()`.
- Hosted LLM owner URA selection is expressed as an aggregate snapshot query.
- Duplicate hosted LLM identity rows do not produce arbitrary health-owner selection.
- Architecture convergence gate prevents reintroducing split health scan reads.
