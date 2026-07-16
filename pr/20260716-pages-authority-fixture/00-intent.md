# Pages Authority Fixture

## Concrete use case

The Pages manifest test installs one process-wide dispatch registry. Its
authority context must be self-contained so parallel tests cannot change its
Device realm through persisted local credentials.

## Owner boundary

The fixture binds a fixed Device authority and a hosted user Agent from the
same realm. The registry therefore proves the Pages API path without reading
ambient daemon identity state.

## Public compatibility

No runtime route, descriptor, or persisted identity changes. This only makes
the test fixture deterministic.
