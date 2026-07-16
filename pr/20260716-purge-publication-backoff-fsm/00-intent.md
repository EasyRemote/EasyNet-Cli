# Intent

## Goal

Close the Agent purge publication retry off-by-one in the durable outbox FSM.
After a drain claim fails, a backoff of N epochs must defer N complete future
scheduled drains before the entry is claimable again.

## Non-goals

- Do not redesign the whole purge/revoke implementation in this slice.
- Do not touch unrelated SDK/provider, runtime-lifecycle, or formatting churn.
- Do not change public CLI command behavior.
- Do not introduce wall-clock leases or compatibility fallbacks.

## Acceptance Criteria

- The normative Agent purge publication FSM spec is committed as the source for
  tombstone/revoke retry, reconciliation, and Hub durable revoke behavior.
- `AgentPurgePublicationRetry::record_failure` computes the first eligible
  epoch after the deferred epoch window, not at the start of that window.
- A focused unit test proves first failure at drain epoch 10 cannot be reclaimed
  at epoch 11 and can be reclaimed at epoch 12.
- Existing purge reconciliation, durable revoke, and architecture gates remain
  green.
