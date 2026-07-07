# Java/JVM Daemon SDK

Java/JVM is a P1 facade for enterprise and Android-adjacent integrations.

Current status: Runtime Core plus Health plus Directory + Identity plus Receipt plus Events seam. The package exposes dependency-free Java
objects for typed SDK errors, feature discovery, complete Invocation draft
construction, `PreparedInvocation`/`SigningMaterial`/`SignedInvocation`
prepare-sign-submit seams over an injected transport, `RuntimeClient` dispatch,
and bounded stream/bidi retained-history state. `AsyncRuntimeClient` exposes
`CompletableFuture`-based invocation/open/cancel methods, and stream/bidi
handles implement `Iterator` over the same bounded lifecycle state. Health DTOs
and `HealthClient` decode shared health and diagnostics payloads over injected
transports. Directory + Identity DTOs and clients build read-model and
directory subscription carrier requests, project pages/resolved refs and
subscription state, open subscription stream handles over injected transports,
and delegate descriptor projection without SDK-owned route selection or fan-out.
Receipt DTOs and `ReceiptClient` build fetch carriers, project summary
receipts, and require explicit receipt URA plus hash facts for causal refs. It
also exposes Events request/filter/cursor/frame/page DTOs and `EventClient`
carrier/projection/stream methods over injected transports without SDK-owned
event fan-out.
does not include a daemon or C ABI provider, generated DTOs for every profile,
provider-backed transport evidence, or product cutover evidence. Maven package
metadata exists for this seam, is verified by
`tools/scripts/check-java-sdk-seam.sh`, and declares only directly exercised
Runtime Core cases through
`sdk/conformance/runner/java-action-adapter-report.json`.

This package must not import generated Axon wire types or daemon internals in
public APIs. See `../SDK_PARITY.md` before claiming provider-backed or package
stable support.
