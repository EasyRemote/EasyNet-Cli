# Intent: cancellation terminal retention idempotency gate

## Goal

Close the A72 regression class by making cancellation terminal retention an
explicit architecture invariant.

The concrete failure mode is repeated observation of the same terminal
invocation lifecycle. If every observation appends another eviction token, the
bounded retention queue can later evict a duplicate and remove the current
terminal map entry.

## Scope

- Guard `InvocationCancellationRegistry` terminal retention.
- Require one retained queue token per terminal lifecycle key.
- Require the terminal transition to enter through the retention helper.
- Add a convergence self-test fixture that catches direct duplicate
  `terminal_order.push_back` behavior.

## Non-goals

- No cancellation protocol changes.
- No public API changes.
- No retention capacity change.
