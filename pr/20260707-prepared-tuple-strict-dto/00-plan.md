# Prepared Tuple Strict DTO Plan

## Objective

Remove obsolete Go and Python `PreparedInvocation` tuple normalizers now that
the Rust/C ABI projection emits the SDK-facing invocation draft JSON instead of
daemon materialization fields.

## Current Defect

The SDK facades still carried defensive stripping logic for prepared tuple
fields such as `args_digest_hex`, `descriptor_hash_hex`, `schema_hash_hex`,
`canonical_hash_hex`, and `expires_at_unix_ms`. That logic was necessary while
the FFI projection leaked daemon/Axon materialization into the tuple, but it is
now duplicate permissiveness after the ABI projection was fixed.

## Steps

1. Keep `PreparedInvocation` decoding routed through strict `InvocationDraft`
   decoding.
2. Remove the Go materialization-field stripping path.
3. Remove the Python materialization-field stripping path.
4. Verify signing tests and live SDK smokes.
