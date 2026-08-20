# Invariants

- Axon `LocalRuntime` remains the owner of stream admission, progress frames,
  terminal state, and receipts.
- Product directory stream code owns only JSON domain values and live presence
  event projection.
- Runtime-registered route closures must not hold a strong reference to the
  daemon service, presence registry, or route lifecycle owner.
- Active directory stream bridge tasks must observe daemon route lifecycle
  teardown and close boundedly.
- No direct `InvokeStreamChunk` helper path may be reintroduced for exact
  directory routes.
- Public snapshot, delta, resume-sequence, and heartbeat behavior remains
  compatible.
