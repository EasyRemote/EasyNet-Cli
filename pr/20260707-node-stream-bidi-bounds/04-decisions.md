# Decisions

1. Node now exposes bounded retained stream and bidi histories as facade state,
   not as daemon protocol state.
2. `max_buffered_events` and `max_buffered_frames` from open metadata override
   the default named limits when positive; zero or absent values use the SDK
   defaults.
3. Overflow produces a typed `ADMISSION_DENIED` terminal/error projection with
   `after_backoff` retry and `callback_queue_overflow` details.
4. The shared `stream/backpressure_bound` conformance case remains undeclared
   for Node because the case requires callback queue overflow and daemon
   wire-code projection, not just facade retained-history bounds.
