# Invariants

## Lifecycle State Machine

```text
LocalRuntime stream admitted
  -> projection sends admission/progress chunks
  -> terminal frame observed: project terminal receipt and close
  -> frame error observed: project canonical terminal error and close
  -> downstream sender closed before terminal: cancel Axon handle, await
     finalization, then close projection task
```

## Required Properties

1. A terminal local stream frame must not be converted into cancellation.
2. A canonical frame error that already finalized the invocation must not be
   overwritten by consumer-disconnect cancellation.
3. A non-terminal send failure means the gRPC consumer is gone; the daemon must
   call `StreamingInvocationHandle::cancel` and wait for canonical finalization.
4. Cancellation failure is not a wire error because the consumer is already
   gone, but it must be observable through the daemon operation log.
5. No legacy or fallback stream path is introduced.
