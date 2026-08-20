# Node Invocation Handle Seam Boundary Proof

## Ownership

Runtime Core owns submitted invocation observation, cancellation requests, and
terminal event projection. The Node SDK owns only typed DTOs and language
facade delegation over an injected transport.

## Call Path

```text
Node consumer
  -> RuntimeClient.submitSigned(...)
  -> InvocationHandle
  -> RuntimeClient.awaitResult/cancel/events/closeHandle
  -> injected RuntimeTransport
  -> daemon/runtime observation endpoint
```

## Rejected Designs

- Local polling loops in the SDK: rejected as daemon transport/provider
  ownership.
- Mutating a handle after cancel or await to invent terminal state: rejected
  because daemon projections are authoritative.
- Declaring `invocation/handle_terminal_monotonicity` for Node in this slice:
  rejected until Node also exposes evidence for prepare/sign in the same shared
  action sequence.
