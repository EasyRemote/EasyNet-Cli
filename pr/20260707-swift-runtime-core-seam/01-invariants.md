# Swift Runtime Core Seam Invariants

1. Swift exposes generic runtime concepts only: client, feature set, typed error, Invocation tuple/draft, runtime transport, stream handle, and bidi session.
2. Public Swift APIs use URA naming only. Legacy address spelling is forbidden.
3. Public Swift APIs do not expose generated wire types, daemon internals, or product-specific lifecycle concepts.
4. Swift lifecycle state is explicit: clients, streams, and bidi sessions reject use after close and expose cancel/terminal state where relevant.
5. Stream and bidi retained histories are bounded and terminate with typed backpressure state once capacity is exceeded.
6. Runtime dispatch is provider-injected only. The seam does not open daemon sockets, load the C ABI, or reimplement protocol algorithms.
7. The seam preserves the shared capability-state model by claiming only `seam`; provider-backed and cutover-ready remain unsupported.
