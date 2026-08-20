# Boundary Proof

## SDK Boundary

The SDK fields are generic runtime facts:

- `issuer_ura`
- `subject_ura`
- `audience`
- `scopes`
- `issued_at_ms`
- `expires_at_ms`
- `signature_base64`

These names do not encode EasyNet, EasyRemote, backend, user-session, or
product receipt concepts.

## Runtime Boundary

Rust admission validates the metadata against the invocation envelope and
runtime audience/scope rules. SDKs parse, validate shape, and materialize typed
metadata without owning admission policy.

## Language Parity

Go and Python use the same schema, fixture, and conformance runner contract.
The change is a single shared runtime model, not a language-specific SDK fork.

