# API Contract

## Durable Outbox

`AgentPurgePublicationEntry::claim(drain_epoch, force_backoff, claim_id)`
returns:

- `Ok(true)` only when the entry is claimable in the supplied epoch.
- `Ok(false)` when the entry is in a non-claimable retry state.
- `Err` for invalid claim identity, zero epoch, zero fence, or corrupt state.

`force_backoff=true` remains an explicit recovery/test override. Normal
scheduled drains use `force_backoff=false`.

## Failure Recording

`record_claim_failure(claim_id, now_unix_ms, error)`:

- requires the current durable claim to match `claim_id`;
- increments the finite attempt budget;
- enters `reconciliation_required` at the terminal attempt;
- otherwise enters `backing_off { eligible_drain_epoch }` where
  `eligible_drain_epoch = claim_drain_epoch + checked_delay_epochs(attempt) + 1`.

## Public Behavior

No public CLI or daemon API shape changes. The externally visible change is
bounded retry behavior: failed purge publication drains wait the intended
logical backoff before reattempting network publication.
