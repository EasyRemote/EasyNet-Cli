# Invariants

- Product callers still receive `DaemonFrameStream` / `StreamHandle` DTOs, not
  gRPC iterators or generated protocol types.
- Direct stream queueing is bounded by a named constant.
- Each direct stream event must carry a positive sequence before it reaches the
  public SDK state machine.
- A daemon stream that ends without a terminal frame is a protocol mismatch.
- gRPC cancellation, deadline, and unavailable errors map to typed `SDKError`.
- Direct bidi, prepare, submit, and handle operations stay explicitly
  fail-closed until their daemon contracts are implemented.
