Invariants
==========

1. Stream cancellation projection uses only the canonical public fields:
   `stream_id`, `cancelled`, `state`, and `terminal`.
2. Bidi frames use the canonical `BidiFrame` shape. Frame-level lifecycle state
   belongs to the session/outcome projection, not to individual bidi frames.
3. Backpressure failures remain bounded and explicit without adding product
   lifecycle fields to canonical SDK frames.
4. Python SDK conformance must load the same canonical Axon Python SDK dependency
   used by public API inventory and parity gates.

