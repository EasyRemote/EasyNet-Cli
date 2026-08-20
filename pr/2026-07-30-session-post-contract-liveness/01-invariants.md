# Invariants

1. Before `SessionEstablished`, down-stream silence is bounded by the admission watchdog.
2. After `SessionEstablished`, down-stream EOF/reset or send-path failure owns teardown, not lack of business frames.
3. Session escalation outbox publication remains gated on the established contract.
4. Post-contract idle UI must not create presence gaps or `TARGET_OFFLINE` false negatives.

