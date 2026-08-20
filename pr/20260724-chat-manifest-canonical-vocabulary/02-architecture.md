# Architecture

The default chat manifest is a product-facing runtime catalog artifact. It is not a migration note.

Layering:

- Manifest owns ability schema metadata.
- Chat handler owns execution semantics.
- Tests own backward-compatibility assertions when public fields must remain stable.

The refactor keeps the public fields but removes historical vocabulary from active catalog metadata.
