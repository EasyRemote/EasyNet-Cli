# Invariants

1. Runtime Core owns stream and bidi lifecycle state.
2. Node stream/bidi queues remain bounded by named exported constants.
3. Overflow produces a typed terminal backpressure frame/event.
4. Overflow uses canonical retry/detail values: `after_backoff`,
   `callback_queue_overflow`, and `RESOURCE_EXHAUSTED`.
5. Local facade queue overflow does not claim daemon wire backpressure provider
   support.
6. No product HTTP, WebSocket, SSE, or bridge policy is introduced.
7. No non-URA naming and no retired input-name compatibility is introduced.
