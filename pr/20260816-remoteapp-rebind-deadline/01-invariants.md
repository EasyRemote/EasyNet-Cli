# Remoteapp Rebind Deadline Invariants

1. Every `Rebinding` phase has a single start timestamp and a derived deadline.
2. A pending media rebind must terminally become `TARGET_REBOUND` or `TARGET_REBIND_FAILED` no later than the automatic rebind deadline.
3. A post-loss rebind attempt without a matching explicit policy must terminally become `TARGET_REBIND_FAILED` no later than the automatic rebind deadline.
4. Deadline expiry is owned by the target binding state machine; callers may tick it, but callers must not duplicate transition rules.
5. The monitor must evaluate deadline expiry on every tracked-session tick, even when host observation yields no target event.
6. Lifecycle evidence must use URA terminology and must not contain duplicated JSON keys that obscure receipt/event meaning.
7. Expiry must preserve fail-closed behavior: input remains disabled, session rebinding is rejected, and stale media sources are not silently reused.
