# Axon Receipt Chain Verification Plan

## Objective

Advance the Receipt profile from single full-receipt signature verification to
Axon-backed receipt-chain verification with receipt-level parent DAG closure
checks, without copying Axon's full offline invocation-bundle verifier into
EasyNet-Cli.

## Invariants

- The SPEC remains unchanged.
- Axon remains the authority for receipt canonical bytes and signature
  verification.
- EasyNet-Cli may project SDK DTOs, check bounded lifecycle continuity, and
  require every `parent_receipts[*].receipt_hash_hex` to resolve inside the
  verified receipt set. It must not reimplement Axon's full invocation-envelope
  causal DAG verifier from `sdk/rust/src/bin/verify.rs`.
- Summary-shaped receipt facts remain conservative and never become
  cryptographic proof.
- Chain verification input must use explicit public keys; missing key material
  must fail closed.
- RFC-007 receipt URA construction remains unresolved and opaque.

## Implementation Steps

1. Extend `project_receipt_chain_verification` to detect full Axon
   `ReceiptJson[]` plus `public_keys`.
2. Reuse `ReceiptJson::to_body`, hosted-attestation projection,
   `verify_receipt_signature_with_hosted`, and
   `canonical_receipt_bytes_with_hosted` for each receipt.
3. Check per-invocation index monotonicity and `prev_receipt_hash_hex` against
   recomputed canonical receipt hashes.
4. Check receipt-level parent DAG closure and acyclicity over verified parent
   receipt hashes.
5. Preserve the existing summary-only hash-continuity projection as the
   conservative fallback.
6. Update parity notes to move receipt-chain/DAG closure verification out of
   the remaining gap while leaving full invocation-bundle causal proof and
   RFC-007 receipt URA construction explicit.
7. Run targeted Rust/C ABI tests plus Go/Python receipt regressions and hygiene
   checks.

## Boundary Proof

This is not a new receipt protocol implementation. The cryptographic checks use
Axon's exported canonical bytes and signature verifier. EasyNet-Cli owns only
request-shape detection, key-map adaptation, bounded lifecycle ordering,
parent-hash closure checks, and SDK DTO projection. Cross-invocation
invocation-envelope causal DAG traversal remains Axon-owned until a stable
library API exists.

## Verification Plan

- `cargo test receipt_contract --lib`
- `cargo test receipt_verify --lib`
- `go test ./... -run Receipt`
- `uv run python -m unittest discover tests`
- `cargo fmt --check`
- `git diff --check`
- `git diff -- docs/spec/daemon-sdk-requirements-v1.md`

## Decisions Log

- Full chain verification uses wrapper items of the form
  `{ "receipt_ura": "...", "receipt": <ReceiptJson> }` so EasyNet-Cli does not
  fabricate receipt URAs while RFC-007 remains unresolved.
- The verifier is intentionally scoped to one invocation's ordered receipt
  chain. Cross-invocation causal DAG verification stays explicit remaining
  work because Axon's DAG verifier is currently implemented in the Axon verifier
  binary rather than a stable library API.
- Summary-only chain projection remains conservative and keeps the existing
  `daemon_receipt_chain_continuity` method.

## Verification Result

- PASS: `cargo test receipt_contract --lib`
- PASS: `cargo test receipt_verify --lib`
- PASS: `go test ./... -run Receipt`
- PASS: `uv run python -m unittest discover tests`
- PASS: `cargo fmt --check`
- PASS: `git diff --check`
- PASS: `git diff -- docs/spec/daemon-sdk-requirements-v1.md` produced no diff.
