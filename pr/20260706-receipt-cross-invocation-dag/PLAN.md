# Receipt Cross-Invocation DAG Plan

## Intent

Promote the daemon SDK Receipt profile from single-invocation receipt-chain
projection to provider-backed cross-invocation receipt DAG closure for full
Axon audit bundles.

## Boundary Proof

- Axon remains the cryptographic verifier and canonical receipt owner.
- EasyNet-Cli SDK only projects daemon/Axon verification facts into stable
  Receipt profile DTOs.
- Go and Python facades parse the projection; they do not verify signatures or
  construct receipt URAs.
- RFC-007 receipt URA construction remains unresolved and must stay listed as
  incomplete.

## Invariants

1. Every receipt in a full audit bundle must pass Axon signature verification
   before it contributes a hash to the DAG closure check.
2. Receipt index and `prev_receipt_hash` continuity is checked independently per
   `invocation_id`, so sibling child invocations may each start at index `0`.
3. Parent receipt edges may cross invocation ids, but every parent hash must
   resolve to a verified receipt in the supplied bundle.
4. Missing, duplicate, self-cyclic, or cyclic parent edges fail verification.
5. Language facades may expose `chain_projection` metadata but must not claim to
   own cryptographic verification.

## Implementation

1. Group verified Axon receipt entries by `invocation_id` for index/hash
   continuity.
2. Keep parent edge closure as a verified-hash DAG check across all receipts in
   the bundle.
3. Update shared conformance expectations from single-invocation chain language
   to cross-invocation provider-backed DAG language.
4. Update Go/Python Receipt tests to accept the new projection metadata.
5. Update parity documentation to remove cross-invocation DAG verification from
   the remaining gaps while preserving RFC-007 receipt URA as incomplete.

## Verification

- Rust receipt contract focused tests.
- Go Receipt/conformance focused tests.
- Python Receipt/conformance focused tests.
- SDK scaffold and parity matrix self-tests.
