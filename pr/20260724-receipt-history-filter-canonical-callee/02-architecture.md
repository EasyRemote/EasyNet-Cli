# Architecture

## Boundary

`invocation.history.*` is a runtime governance surface over the Axon invocation ledger. Its filters must reflect canonical ledger fields.

## Layering

- SDK: product-neutral receipt filter DTOs.
- CLI facade: preserves user-facing ergonomics and lowers to SDK/runtime fields.
- Daemon runtime: accepts only canonical wire fields and owns ledger query construction.

## Ownership

Directory concepts such as "agent" remain in directory modules. Receipt-history modules own invocation tuple filtering only.
