# API Contract

No public API or wire behavior changes.

Stable production constants:

- principal lifecycle ability names;
- access-control ability names;
- receipt/history ability names;
- runtime-admin ability names.

Test-only constants:

- `*_PROFILE`;
- `*_ROUTE_MANIFEST_SHA256`.

The test-only constants remain available to `#[cfg(test)]` modules that verify
manifest freshness.
