# Intent

## Root fork

`meta.teach` reopens `agents.json` in two registry-only read paths even though
`AgentAggregateRepository` owns durable Agent registry projections.

## Objective

Route owner-manifest resolution and post-forget transaction recovery through
the registry-only projection owner. The descriptor-transfer lifecycle keeps its
existing durable state machine; only its registry input boundary changes.

## Public behavior

- Unknown owners and missing workspace roots retain their precise messages.
- Registry persistence failures retain their raw error surface.
- Forget recovery remains terminal when the learner registry row has been
  removed, and remains retryable only when runtime convergence is unavailable.
