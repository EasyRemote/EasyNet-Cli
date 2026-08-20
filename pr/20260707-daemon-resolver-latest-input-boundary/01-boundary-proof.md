# Boundary Proof

## Ownership

The SDK owns public Directory DTO fields such as `query_name`; the daemon
resolver owns the internal `namespace.resolve` ability wire input. The current
daemon wire field names are `queryName`, `abilityName`, `realmHint`, and
`qtype`.

## Invariant

There is one translation boundary:

```text
SDK DirectoryResolveRequest.query_name
  -> SDK carrier builder
  -> daemon namespace.resolve args.queryName
  -> DaemonRouteResolver
```

The daemon resolver must not also accept SDK DTO field names directly. Accepting
both shapes makes the lower layer a compatibility decoder and weakens the
latest-only input boundary.

## Non-Goals

- Do not rename the daemon `namespace.resolve` wire contract in this change.
- Do not add compatibility aliases for older callers.
- Do not move route selection into SDK facades.
