# Architecture

## Boundary

Generated Rust route modules contain two classes of data:

- runtime constants: ability names consumed by daemon code;
- proof metadata: manifest profile and manifest digest consumed by tests.

The runtime constants belong in production builds. Proof metadata belongs in
`#[cfg(test)]` so it does not inflate runtime module surface or produce dead-code
warnings.

## Change

Update `provider_routes/route_generator.py` so `rust_source` emits
`#[cfg(test)]` before the profile and digest constants. Regenerate all Rust
route modules that use this generator.
