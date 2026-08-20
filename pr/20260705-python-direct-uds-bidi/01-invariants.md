# Invariants

1. Axon remains the protocol truth.
   - Frame 0 is `InvokeBidiUp(sequence=0, envelope_open=...)`.
   - `EnvelopeOpen` reuses the same envelope, caller signature, arguments,
     metadata, target descriptor, and content envelope as unary/stream invoke.
   - `EnvelopeOpen.target` uses the current typed `AbilityTarget` oneof and
     carries the descriptor ref opaquely; Python must not fall back to legacy
     target fields or split descriptor refs.
   - Bidi streams must be explicitly declared and non-empty, matching the Rust
     daemon SDK path.

2. The Python SDK must not own the bidi lifecycle state machine.
   - `BidiSession` validates state transitions, frame ordering, terminal close,
     cancel, and buffer accounting.
   - The direct transport is a `BidiTransport` implementation only.

3. Sequence projection is stable at the SDK facade boundary.
   - Axon frame 0 stays internal to the transport.
   - User-visible SDK frames are positive and strictly ordered.

4. Backpressure and shutdown are bounded.
   - The inbound down-frame queue has a finite capacity.
   - Local close/cancel stops the gRPC iterator and wakes blocked transport
     paths without leaking reader threads intentionally.

5. Unsupported carrier-v1 dispatch frames must be surfaced explicitly instead
   of silently pretending to understand them.
