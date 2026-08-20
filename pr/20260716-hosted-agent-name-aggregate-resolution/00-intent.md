# Intent

## Goal

Move hosted-Agent display-name resolution onto the Agent aggregate read owner for runtime and CLI surfaces that resolve a human-facing Agent name into a canonical Agent URA.

## Non-goals

- Do not refactor the full `meta.teach` transaction state machine in this slice.
- Do not change public CLI flags or daemon Invocation payloads.
- Do not change hosted Agent publication or lifecycle mutation persistence.

## Acceptance Criteria

- `AgentAggregateSnapshot` exposes a typed hosted-Agent-by-name lookup.
- Smaller runtime/CLI callers no longer load `local-agents.json` only to call `lookup_hosted_agent_by_name`.
- Name ambiguity remains a hard error and is owned by the aggregate read model.
- Public error messages remain actionable for missing and invalid hosted Agent URAs.
- Architecture convergence gate prevents reintroducing direct hosted-name file lookup in migrated surfaces.
