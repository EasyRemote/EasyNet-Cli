# Decisions Log

## 2026-07-27

- Chose ingress-proof refactoring over changing public behavior because the current product Docker E2E already proves clean stream/bidi behavior, while the remaining architecture smell is a boolean authority carrier inside daemon dispatch.
- Modeled bootstrap separately from external signed ingress so future bootstrap-specific policy cannot accidentally inherit public signed behavior through a shared `false` branch.
- Kept local-system envelope verification inside the issuer even after adding the ingress enum. The enum proves dispatcher classification; the envelope check proves the canonical tuple still carries `_system.local`.
