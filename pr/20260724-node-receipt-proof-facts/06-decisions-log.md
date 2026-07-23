# Decisions Log

## 2026-07-24

- Treat Node field-shape-only receipt validation as an active SDK architecture fork.
- Reuse the canonical `authority_proof_expected_hash` semantics: payload hash first, binding projection hash when payload is empty.
- Keep the proof-fact validator internal to the Node SDK public surface. The canonical authority bytes are validation material, not a product API.
- Require hosted signer receipts to carry host attestation and self-signed receipts to omit it, matching Go/Python/Java receipt topology.
