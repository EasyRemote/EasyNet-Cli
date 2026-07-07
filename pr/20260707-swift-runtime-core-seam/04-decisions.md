# Swift Runtime Core Seam Decisions

## Decision 1: Seam Before Provider

Swift now has a Runtime Core seam instead of remaining a placeholder. The implementation stops at dependency-free public DTOs and injected transports because provider-backed support requires a stable daemon/C ABI adapter and conformance report. Swift Package Manager metadata exists only as the package boundary for this seam.

## Decision 2: Generic Feature Discovery

Feature discovery exposes generic profile, symbol, and protocol-bridge availability fields. Protocol-specific public naming was avoided so the SDK remains a canonical runtime model rather than a wire-protocol facade.

## Decision 3: Canonical Invocation Naming

The Swift invocation tuple exposes `descriptorRef` to match the canonical runtime field and avoid ambiguous descriptor naming. The builder remains fluent and requires every tuple component before producing an `InvocationDraft`.

## Decision 4: Bounded Retained History

Swift stream and bidi handles retain a bounded local history and transition to terminal backpressure state when capacity is exceeded. This makes lifecycle state explicit and prevents unbounded memory growth in facade consumers.
