# Decisions Log

## 2026-07-07

- Chose Runtime Core DTO validation rather than signing-layer-only validation because the SPEC requires `Build` to reject invalid nonce length and because unary, stream, bidi, and prepare must share one draft validity model.
- Kept the public API unchanged; this is a stricter validation of the existing contract, not a schema change.
