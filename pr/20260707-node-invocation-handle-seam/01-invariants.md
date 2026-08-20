# Invariants

1. Submitted invocation observation is Runtime Core behavior, not a product
   lifecycle or backend polling model.
2. `InvocationHandle` owns only submitted observation identity, state snapshot,
   event cursor, optional terminal result projection, and client binding.
3. Await, cancel, events, and free-handle delegate through injected transport
   methods; the Node SDK does not poll daemon sockets directly.
4. Terminal state is monotonic: a handle snapshot may contain at most one
   terminal event and terminal snapshots must be consistent with terminal
   events.
5. Cancellation does not rewrite a completed terminal state in the SDK; the
   daemon result/cancel/event projections remain authoritative.
6. No legacy aliases are accepted for handle IDs, event fields, or cancellation
   fields.
