# Axon Receipt Signature Verification Plan

## Objective

Replace the absolute "receipt verification is declaration-only" gap with a
narrow, delegated Axon verifier path for full single-receipt audit bundles.

The SDK must not invent receipt canonical bytes, signature checks, hosted
attestation rules, or public-key resolver semantics. It may only adapt SDK JSON
inputs into Axon's verifier APIs and project the result into the existing
ReceiptVerification DTO.

## Invariants

- The SPEC remains unchanged.
- Summary-shaped receipts remain non-cryptographic projections.
- `verified=true` is allowed only when Axon verifier APIs accept the receipt
  signature.
- Verification requires a full Axon `ReceiptJson` audit bundle and explicit
  public key map.
- Chain/DAG verification and RFC-007 receipt URA construction remain
  incomplete; this patch must not claim them.

## Implementation Steps

1. Add an inline KeyResolver adapter over explicit public keys.
2. Parse full Axon audit-bundle receipts with `easynet_axon::invocation::ReceiptJson`.
3. Delegate signature verification to
   `verify_receipt_signature_with_hosted`.
4. Project cryptographic verification evidence through the existing
   ReceiptVerification DTO metadata.
5. Preserve the conservative summary projection path unchanged.

## Boundary Proof

The SDK does not implement canonical receipt bytes, Ed25519 verification rules,
hosted receipt attestation, or receipt body parsing semantics. Those remain in
EasyNet-Axon. The SDK only owns JSON adaptation, typed error/projection shape,
and explicit refusal to upgrade summary-only data to cryptographic evidence.

## Verification Plan

- `cargo test receipt_contract --lib`
- `cargo test receipt_verify --lib`
- `go test ./... -run Receipt`
- `go test ./...`
- `uv run python -m unittest discover tests`
- `git diff --check`
- `git diff -- docs/spec/daemon-sdk-requirements-v1.md`

## Verification Result

- PASS: `cargo test receipt_contract --lib`
- PASS: `cargo test receipt_verify --lib`
- PASS: `go test ./... -run Receipt`
- PASS: `go test ./...`
- PASS: `uv run python -m unittest discover tests`
- PASS: `git diff --check`
- PASS: `git diff -- docs/spec/daemon-sdk-requirements-v1.md` produced no diff.
