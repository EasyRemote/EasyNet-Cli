# Invariants

1. Exact stream route registration must happen before invocation listeners are
   reachable, alongside exact unary route registration.
2. `InvokeStream` must not direct-dispatch `DaemonStreamRoute` variants from
   the tonic service match arm.
3. Stream route product logic may build payload frames, but runtime admission,
   invocation id, admission receipt, terminal receipt, and stream lifecycle
   projection belong to `LocalRuntime`.
4. Direct product stream helpers must not be public dispatch endpoints once the
   runtime adapter owns exact stream route dispatch.
5. The convergence gate must reject reintroducing `dispatch_subscribe_directory_*`
   direct calls from `Invocation::invoke_stream`.
6. Existing unrelated dirty changes in federation read models and unary route
   registration visibility are not part of this slice unless required by the
   runtime stream refactor.
