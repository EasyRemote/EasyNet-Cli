# Prepared Tuple Materialization Plan

## Objective

Make Go and Python `PreparedInvocation` decoders tolerate daemon/Axon
materialization fields in the prepared `tuple` object without weakening strict
`InvocationDraft` decoding.

## Current Defect

Prepared material can carry daemon-owned fields such as `args_digest_hex`,
`descriptor_hash_hex`, `schema_hash_hex`, `canonical_hash_hex`, or
`expires_at_unix_ms` next to the SDK-facing invocation tuple. The SDK draft
decoder correctly rejects unknown invocation fields, but prepared decoding needs
to normalize this daemon materialization before handing the tuple to the draft
decoder.

## Steps

1. Keep `InvocationDraft` strict for public SDK input.
2. Normalize only the prepared tuple path in Go and Python.
3. Extend Go/Python signing tests with daemon materialization fields in the
   prepared tuple fixture.
4. Re-run Go/Python signing and live smoke gates.
