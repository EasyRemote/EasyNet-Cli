# Architecture

## Layering

- `support::async_bridge` owns the cross-cutting runtime bridge recipe.
- Domain modules choose an explicit `SyncBridgeRuntimePolicy` at the call site.
- Domain modules do not implement private `Handle::try_current` ladders.

## Boundary proof

The previous `NoRuntimeFallback` type named the policy as a fallback from a
missing runtime. That implied a secondary compatibility path. The replacement
type names the same state machine as a sync bridge runtime policy:

1. multi-thread ambient runtime;
2. current-thread ambient runtime that cannot be re-entered;
3. no ambient runtime.

Each state has a deliberate policy outcome.
