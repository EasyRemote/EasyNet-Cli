# Architecture

`RuntimeClient` owns local request admission for descriptor-ref resolution before it crosses the provider transport seam.

High-level providers (`RuntimeAbilityClient`, receipt provider, descriptor provider) already supply caller and subject. This change moves the invariant into the shared lower client so direct callers cannot bypass provider-backed request completeness.

Layering:

- SDK RuntimeClient: validates generic request completeness and provider-backed identity facts.
- Runtime providers/transports: resolve descriptor refs only after receiving a complete admitted request.
- Daemon/Rust provider: remains the authoritative descriptor catalogue implementation and continues enforcing provider-specific subject shape.
