# Invariants

1. Drain epochs are monotonic logical clocks; retry eligibility must never use
   wall-clock time for ownership or takeover.
2. `delay_epochs` counts complete future drains to skip. The next eligible
   epoch is `failure_drain_epoch + delay_epochs + 1`.
3. A claimed entry has exactly one owner, identified by `claim_id`, and only that
   owner may record failure or advance stages.
4. Retry counters are finite. Exhaustion enters `reconciliation_required`, and
   scheduled drains cannot claim that state.
5. Delivery fences are monotonic per outbox entry. A failed retry must not
   rewind the next fence or reuse a prior claimed fence.
6. Tombstone publication and Hub revoke remain separate stages; stage progress
   resets attempts only while retaining the active durable claim.
