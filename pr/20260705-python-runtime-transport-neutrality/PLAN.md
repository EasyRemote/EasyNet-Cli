# Python Runtime Transport Neutrality Plan

## Goal

Remove EasyRemote product naming from Python Runtime Core transport adapter objects and expose product-neutral daemon SDK names without changing the underlying unary, stream, bidi, signing, or wait-state behavior.

## Boundary Proof

- Runtime transport adapters are daemon SDK infrastructure. They must not be named after a product facade.
- EasyRemote may consume these adapters from its own repository, but product-specific result shaping and decorators must not define SDK object ownership.
- The slice only renames the Runtime transport adapter family and typed stages; it does not move product cutover audits, daemon lifecycle facades, admin/profile bridges, or receipt product shims.
- The behavior remains the same: complete Invocation dispatch, signed submit, stream value projection, bidi lifecycle, and bounded unary wait/retire state.
- No spec text is changed, and URA terminology remains unchanged.

## Implementation Slices

1. Rename public transport adapter objects in `sdk/python/easynet_sdk/transport.py`.
2. Update package exports and Python transport tests to use neutral names.
3. Adjust typed error stages away from product-specific strings.
4. Run focused Python tests, full Python SDK tests, scaffold, formatting, diff, and terminology checks.
