# Java/JVM Runtime SDK

The Java package is a dependency-free generic Runtime Core seam. Its public
model is limited to:

- feature discovery and explicit client lifecycle;
- typed SDK errors and stable error classes;
- runtime health and diagnostics;
- complete Invocation tuple construction;
- prepare, caller-sign, submit, and invocation-handle state;
- delegated and session authority metadata;
- bounded stream and bidirectional-session lifecycle;
- synchronous and `CompletableFuture` runtime clients.

`InvocationResult` preserves runtime-provided receipt facts as an opaque map. It
does not expose a receipt-history client or interpret product receipt policy.

Downstream workflow profiles are deliberately absent. Product administration,
gateway, application lifecycle, translation layers, directory views,
identity projections, event feeds, host binding, orchestration, publication,
receipt-history pages, page/model/file helpers, and wrapper behavior belong to
downstream products. The Java SDK provides no aliases or empty transport
placeholders for those surfaces.

The package currently has no bundled runtime-host transport or C ABI provider.
`tools/scripts/check-java-sdk-seam.sh` compiles all sources with
`javac -Xlint:all -Werror`, runs the Runtime Core state-machine tests, and checks
the source boundary.
