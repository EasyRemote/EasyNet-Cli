# Invariants

## Receipt proof facts

- `authority_proof.proof_hash_hex` must equal SHA-256 of non-empty `proof_payload_base64` bytes.
- If `proof_payload_base64` is empty, `authority_proof.proof_hash_hex` must equal SHA-256 of canonical authority-binding projection bytes.
- `authority_proof.binding` must match `authority_binding`.
- `authority_proof.issuer` must match `callee_binding`.
- Hosted signer receipts require host attestation; self-signed receipts must not carry host attestation.

## Runtime model

- Node remains a language implementation of the canonical runtime model, not a product-specific SDK.
- Runtime receipt lifecycle remains fail-closed and terminal-state aware.
- No URI terminology is introduced.
