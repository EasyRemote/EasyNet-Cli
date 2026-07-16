# Intent

## Goal

Move namespace route resolver hosted-Agent placement reads onto the Agent aggregate owner.

## Non-goals

- Do not change public route response shape.
- Do not change presence or Hub projection routing semantics.
- Do not migrate unrelated route resolver authorities in this slice.

## Acceptance Criteria

- Route resolver hosted placement loading uses `AgentAggregateRepository`.
- Route resolver consumes `AgentHostedPlacementProjection` instead of `LocalAgentsFile`.
- Hosted placement availability is an explicit state and fails closed on aggregate load errors.
- Production route resolver does not directly load or inspect `local-agents.json` for hosted placement.
- Architecture convergence gate prevents reintroducing the direct file projection.
