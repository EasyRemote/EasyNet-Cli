# Node Stream Bidi Bounds Boundary Proof

## Ownership

The daemon/Axon runtime owns stream and bidi wire frames, terminal protocol
semantics, and transport backpressure. The Node SDK owns only facade state:
async iteration, cancellation wiring, lifecycle flags, and bounded local
history.

## Call Path

```text
RuntimeClient.invokeStream(...)
  -> injected RuntimeTransport.openStream(...)
  -> StreamHandle.receive(...)
  -> bounded history projection

RuntimeClient.openBidi(...)
  -> injected RuntimeTransport.openBidi(...)
  -> BidiSession.send/receive/closeSend/close/cancel
  -> bounded frame history projection
```

## Rejected Designs

- Adding an unbounded `events` or `frames` array: rejected because it violates
  the bounded queue requirement.
- Claiming the shared C ABI callback overflow case: rejected for this slice
  because Node does not implement a C ABI callback queue.
- Dropping old entries silently: rejected because callers need a typed terminal
  signal when the facade cannot preserve its declared history contract.
