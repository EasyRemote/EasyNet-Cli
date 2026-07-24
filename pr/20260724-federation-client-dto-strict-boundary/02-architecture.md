# Architecture

## Boundary

The federation client `ability_contract` module is the typed protocol edge for federation ability responses. It must project canonical receipt facts into runtime-owned types and reject non-canonical shapes.

## Layering

- Core runtime owns canonical receipt validation and typed parsing.
- Product code consumes parsed facts; it must not reinterpret legacy receipt variants.
- Tests pin rejection behavior at the same boundary where the DTOs are deserialized.

## Ownership

This change keeps ownership in the runtime federation client. It does not move product lifecycle or EasyNet/EasyRemote naming into SDK abstractions.
