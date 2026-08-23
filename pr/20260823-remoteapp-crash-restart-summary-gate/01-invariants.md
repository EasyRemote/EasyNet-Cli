# Invariants

- Product completion is a single aggregate claim and must fail closed.
- Child verifiers remain non-claiming; `product_complete_claim` must stay false outside the aggregate gate.
- Coverage booleans are not sufficient proof for lifecycle or recovery behavior.
- Recovery evidence must bind selected Resource URA, session id, descriptor version, lifecycle events, replay/idempotency guards, terminal receipt visibility, and post-recovery media/control viability.
- The aggregate gate validates summaries; it does not duplicate the full raw crash evidence verifier.
