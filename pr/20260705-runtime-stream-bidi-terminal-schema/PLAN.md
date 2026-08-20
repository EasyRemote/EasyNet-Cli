# Runtime Stream/Bidi Terminal Schema Plan

## Goal

Make Runtime Core stream and bidi terminal events explicit schema-shaped SDK projections in the P0 Go and Python facades, instead of leaving callers to infer terminal state from loose frame booleans.

## Boundary Proof

- Runtime Core owns stream/bidi terminal semantics; product facades must not parse EOF, cancel, timeout, transport failure, or receipt-bearing terminal frames as raw transport details.
- The change preserves existing `StreamEvent` and `BidiFrame` APIs while adding terminal projection helpers.
- Terminal projections follow existing `stream-event.schema.json` and `bidi-frame.schema.json` field names: stream/session id, event/frame type, sequence, payload, error, receipt.
- No daemon protocol or spec text is changed.
- No retired address terminology is introduced.

## Implementation Slices

1. Add Python `StreamTerminalEvent` and `BidiTerminalFrame` DTOs plus handle-level accessors.
2. Add Go `StreamTerminalEvent` and `BidiTerminalFrame` DTOs plus handle-level accessors.
3. Extend the shared stream/bidi conformance case and Python/Go conformance tests.
4. Update SDK parity docs.
5. Run Python, Go, C ABI Go, cargo fmt, scaffold, diff, and terminology checks.
