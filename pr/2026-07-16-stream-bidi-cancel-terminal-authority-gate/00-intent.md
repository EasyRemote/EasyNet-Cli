# Intent

Close the SDK facade side of the stream/bidi cancellation terminality fork.

Concrete use case: a caller invokes `StreamHandle.cancel` or
`BidiSession.cancel` through the SDK. At the current provider boundary this is
only a transport/resource cancellation request. The SDK must not accept a
provider-local `Cancelled + terminal=true` payload as runtime lifecycle proof;
terminality belongs to the canonical runtime terminal frame/receipt path.

Expected effect: product consistency and architecture convergence. Public
method names stay compatible, but their accepted cancel outcome is narrowed to
the already-supported `CancelRequested, terminal=false` shape.
