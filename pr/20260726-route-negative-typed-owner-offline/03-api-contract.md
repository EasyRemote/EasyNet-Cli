# API Contract

No public API shape changes.

Internal contract:

- `ResolveRouteFailureKind::Generic` means transport mapping follows `NegativeReason`.
- `ResolveRouteFailureKind::OwnerOffline` means the route owner exists as a semantic target but has no online placement; admission maps it to `Unavailable`.

Diagnostic `detail` remains human-readable only and must not be parsed by callers.
