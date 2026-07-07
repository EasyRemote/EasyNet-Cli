# Java Async Runtime Seam Boundary Proof

The async Java seam is an idiomatic facade over the existing Runtime Core seam. It does not introduce a new runtime model, transport owner, daemon lifecycle, or protocol algorithm. `AsyncRuntimeClient` delegates to `RuntimeClient`, which still delegates to injected `RuntimeTransport`.

`RuntimeFuture<T>` exposes Java cancellation state by extending `CompletableFuture<T>`. Cancellation does not claim daemon-side provider cancellation for unary calls; explicit stream and bidi cancellation remains owned by `StreamHandle.cancel` and `BidiSession.cancel`.

`StreamHandle` and `BidiSession` implement `Iterator` directly over their existing bounded retained-history state, so iterator support does not create a second buffering policy.
