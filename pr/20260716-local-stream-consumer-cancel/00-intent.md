# Local InvokeStream Consumer Cancellation

## Concrete Use Case

A local daemon-hosted streaming ability can emit an initial frame and then wait
for more runtime work. If the gRPC `InvokeStream` consumer disconnects after the
first frame, the daemon projection task must cancel the canonical Axon
invocation instead of only dropping the wire sender.

## Owner Boundary

- EasyNet-Cli owns the daemon `InvokeStream` projection from Axon local runtime
  frames into gRPC chunks.
- EasyNet-Axon owns canonical invocation admission, terminal receipts,
  cancellation, and finalization semantics.
- The CLI projection may decide that the transport consumer is gone; it must
  express that lifecycle event by calling the Axon stream handle cancel API.

## Public Behavior

The client-visible API does not change. The only behavior change is internal:
when the client has already stopped consuming a non-terminal local stream, the
underlying canonical runtime invocation is explicitly cancelled and finalized.
