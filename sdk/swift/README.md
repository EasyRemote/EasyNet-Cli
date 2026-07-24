# Swift Runtime SDK

The Swift package is a generic Runtime Core seam for macOS and iOS clients. Its
public model is limited to:

- feature discovery and explicit client lifecycle;
- typed SDK errors and stable error classes;
- runtime health and diagnostics;
- complete Invocation tuple construction;
- prepare, caller-sign, submit, and invocation-handle state;
- delegated and session authority metadata;
- bounded stream and bidirectional-session state machines with `AsyncSequence`.

`InvocationResult` preserves runtime-provided receipt facts as an opaque string
map. It does not expose receipt history or interpret product receipt policy.

Product profiles are deliberately absent. Product administration, gateway,
companion, compatibility, directory views, identity projections, event feeds,
host binding, mission, publication, receipt-history pages, surface/pages, and
wrapper behavior belong to downstream products. The Swift module provides no
aliases or empty transport placeholders for those surfaces.

The package currently has no bundled runtime-host transport or C ABI provider.
`tools/scripts/check-swift-sdk-seam.sh` builds with warnings as errors, runs the
Runtime Core state-machine tests, and checks the source boundary.
