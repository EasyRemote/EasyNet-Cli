# Decisions Log

## 2026-07-26

- Treat legacy lifecycle spelling acceptance as an architectural defect because it creates cross-language SDK divergence and lets receipt carriers bypass the canonical runtime vocabulary.
- Preserve public method names and language-native return projections while making accepted carrier values exact.
- Keep `receipt_type` lowercase because it is the receipt-class discriminator and already distinct from canonical lifecycle `state`.
- Regenerate `sdk/conformance/canonical-public-api.json` and `sdk/conformance/sdk-parity-matrix.json` from source rather than hand-editing source attestation hashes.
