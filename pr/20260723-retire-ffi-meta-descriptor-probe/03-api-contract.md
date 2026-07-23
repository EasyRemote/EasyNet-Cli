# API Contract

## Public Behavior

- The exported FFI descriptor resolver remains available.
- Local runtime system abilities continue to resolve descriptor refs.
- Missing non-local descriptors are reported as descriptor-not-found catalog
  misses.

## Error Behavior

Descriptor resolution must not reclassify catalog misses as:

- caller signer missing,
- route negative,
- owner offline,
- `meta.list_abilities` unavailable,
- timeout.

## Reintroduction Rule

Any future dynamic descriptor catalog must be exposed through a named provider
seam. It must not be implemented by invoking `meta.list_abilities` inside
descriptor resolution.
