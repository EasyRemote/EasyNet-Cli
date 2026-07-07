# Swift Daemon SDK

Swift is a P1 facade for macOS/iOS-adjacent clients.

Current status: Runtime Core plus Health plus Directory + Identity plus Receipt seam. The package exposes dependency-free Swift
types for typed SDK errors, feature discovery, complete Invocation draft
construction, `RuntimeClient` dispatch over injected transports, and bounded
stream/bidi retained-history state. Stream and bidi handles conform to
`AsyncSequence` while preserving the same bounded lifecycle state. Health DTOs
and `HealthClient` decode shared health and diagnostics payloads over injected
transports. Directory + Identity DTOs and clients build read-model and
directory subscription carrier requests, project pages/resolved refs and
subscription state, open subscription stream handles over injected transports,
and delegate descriptor projection without SDK-owned route selection or fan-out.
Receipt DTOs and `ReceiptClient` build fetch carriers, project summary
receipts, and require explicit receipt URA plus hash facts for causal refs. It
does not include a daemon or C ABI provider, generated DTOs for every profile,
provider-backed evidence, or product cutover evidence. Swift Package Manager
metadata exists for this seam, is verified by
`tools/scripts/check-swift-sdk-seam.sh`, and declares only directly exercised
Runtime Core cases through
`sdk/conformance/runner/swift-action-adapter-report.json`.

This package must not import generated Axon wire types or daemon internals in
public APIs. See `../SDK_PARITY.md` before claiming provider-backed or package
stable support.
