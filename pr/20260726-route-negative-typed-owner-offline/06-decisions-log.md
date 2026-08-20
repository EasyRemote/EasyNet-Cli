# Decisions Log

2026-07-26:

- Treat owner-offline as resolver-owned typed failure state, not a diagnostic string.
- Keep public diagnostics unchanged while removing string-based admission semantics.
- Update the SPEC v2 gate to reject admission-layer diagnostic string predicates and require typed `ResolveRouteFailureKind::OwnerOffline`.
