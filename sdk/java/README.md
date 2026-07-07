# Java/JVM Daemon SDK

Java/JVM is a P1 facade for enterprise and Android-adjacent integrations.

Current status: Runtime Core seam. The package exposes dependency-free Java
objects for typed SDK errors, feature discovery, complete Invocation draft
construction, `RuntimeClient` dispatch over an injected transport, and bounded
stream/bidi retained-history state. `AsyncRuntimeClient` exposes
`CompletableFuture`-based invocation/open/cancel methods, and stream/bidi
handles implement `Iterator` over the same bounded lifecycle state. It does not
include a daemon or C ABI provider, generated DTOs for every profile,
provider-backed transport evidence, or product cutover evidence. Maven package
metadata exists for this seam, is verified by
`tools/scripts/check-java-sdk-seam.sh`, and declares only directly exercised
Runtime Core cases through
`sdk/conformance/runner/java-action-adapter-report.json`.

This package must not import generated Axon wire types or daemon internals in
public APIs. See `../SDK_PARITY.md` before claiming provider-backed or package
stable support.
