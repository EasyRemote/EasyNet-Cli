# Decisions Log

## 2026-07-24

- Treat product-visible history/catalog/browser route errors as runtime ingress convergence evidence, not as UI-only failures.
- Preserve strict admission; fix stale caller/subject/authority production instead of adding bypasses.
- Java runtime receipt validation must have a named proof-facts owner. Keeping proof hash, binding projection, signer, and parent receipt semantics embedded in `RuntimeReceipt` recreated a language-specific receipt model.
- Empty proof payload remains valid only when `proof_hash_hex` equals the canonical authority-binding projection hash; non-empty payloads must hash their payload bytes.
