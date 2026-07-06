# Node Publication Seam Boundary Proof

## Ownership

Publication DTO shape and daemon system-ability carrier semantics are owned by
the daemon SDK shared contracts. Node owns only a language facade over those
contracts.

## Call Path

```text
Node caller
  -> PublicationClient
  -> injected PublicationTransport
  -> daemon/C ABI/provider in a future layer
```

The seam has no provider and therefore cannot own execution, plugin policy, or
daemon lifecycle.

## Rejected Designs

- Local ResourceRef URA fabrication: rejected because URA construction belongs
  to daemon/Axon-backed identity helpers.
- Product package/decorator naming: rejected because EasyRemote and future
  product facades own language ergonomics.
- Direct CLI subprocess fallback: rejected because seams must converge toward
  shared runtime providers, not product-specific compatibility paths.
