# Java/JVM Daemon SDK

Java/JVM is a P1 facade for enterprise and Android-adjacent integrations.

Current status: Runtime Core seam. The package exposes dependency-free Java
objects for typed SDK errors, feature discovery, complete Invocation draft
construction, `RuntimeClient` dispatch over an injected transport, and bounded
stream/bidi retained-history state. It does not include a daemon or C ABI
provider, package metadata, generated DTOs for every profile, or product
cutover evidence. It declares only directly exercised Runtime Core cases through
`sdk/conformance/runner/java-action-adapter-report.json`.

This package must not import generated Axon wire types or daemon internals in
public APIs. See `../SDK_PARITY.md` before claiming provider-backed or package
stable support.
