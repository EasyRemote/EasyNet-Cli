# Invariants: cancellation retention

## Idempotent terminal observation

Marking the same lifecycle key terminal more than once must leave exactly one
retention-order token for that key.

## Single queue owner

Only `RegistryState::retain_terminal_key` may append to `terminal_order`.
Lifecycle transition code must call that helper instead of pushing directly.

## Bounded eviction

Eviction may remove a terminal entry only when the popped queue token still
corresponds to a terminal map entry. Duplicate queue tokens must not be able to
delete the current map entry for the same lifecycle key.
