# Intent

## Goal

Move governance teach/acquire/forget hosted-Agent identity authorization onto the Agent aggregate read owner.

## Root Fork

`meta.teach`, `meta.acquire`, and `meta.forget` still resolved hosted Agent display names through `local-agents.json` helper functions even after caller-facing surfaces had moved to `AgentAggregateSnapshot`.

## Non-goals

- Do not redesign the teach grant transaction state machine in this slice.
- Do not change public ability names, request fields, response fields, or CLI behavior.
- Do not change hosted Agent lifecycle mutation persistence.

## Acceptance Criteria

- The Agent aggregate exposes a hosted identity projection that carries canonical Agent URA and persisted signing authority.
- Governance teach authorization consumes aggregate hosted identity projections instead of `LocalAgentsFile` display-name helpers.
- Missing, ambiguous, invalid, and non-Agent hosted identity rows remain hard errors.
- Architecture convergence gate prevents reintroducing direct hosted-name file lookup in governance teach.
