# Swift Daemon SDK

Swift is a P1 facade for macOS/iOS-adjacent clients.

Current status: Runtime Core seam. The package exposes dependency-free Swift
types for typed SDK errors, feature discovery, complete Invocation draft
construction, `RuntimeClient` dispatch over injected transports, and bounded
stream/bidi retained-history state. It does not include a daemon or C ABI
provider, Swift Package Manager metadata, generated DTOs for every profile, or
product cutover evidence.

This package must not import generated Axon wire types or daemon internals in
public APIs. See `../SDK_PARITY.md` before claiming provider-backed or package
stable support.
