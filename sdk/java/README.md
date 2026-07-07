# Java/JVM Daemon SDK

Java/JVM is a P1 facade for enterprise and Android-adjacent integrations.

Current status: Runtime Core plus Health plus Directory + Identity plus Receipt plus Publication plus Host Binding plus Events plus Surface seam. The package exposes dependency-free Java
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
receipts, and require explicit receipt URA plus hash facts for causal refs.
Publication DTOs and `PublicationClient` build daemon-authored resource-ref
requests, package validation inputs, and complete deploy/unpublish Invocation
carriers over injected transports without package inspection or product catalog
state. Host Binding DTOs and `HostBindingClient` build host-stream binding
requests, decode request envelopes, project item/error/terminal frames, validate
output-hash cursor state, and drive explicit readiness/cleanup lifecycle
providers without owning product host process warmth or user-code execution. It
also exposes Events request/filter/cursor/frame/page DTOs and `EventClient`
carrier/projection/stream methods over injected transports without SDK-owned
event fan-out. Surface DTOs and `SurfaceClient` build page, manifest, and health
carriers and project daemon page facts without backend rendering or HTTP route
ownership.
does not include a daemon or C ABI provider, generated DTOs for every profile,
provider-backed transport evidence, or product cutover evidence. Maven package
metadata exists for this seam, is verified by
`tools/scripts/check-java-sdk-seam.sh`, and declares only directly exercised
Runtime Core cases through
`sdk/conformance/runner/java-action-adapter-report.json`.

This package must not import generated Axon wire types or daemon internals in
public APIs. See `../SDK_PARITY.md` before claiming provider-backed or package
stable support.
