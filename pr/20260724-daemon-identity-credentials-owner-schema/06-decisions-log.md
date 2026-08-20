# Decisions Log

## 2026-07-24

- Decision: boot identity must use `load_credentials_optional()` instead of deserializing a local projection.
- Reason: runtime identity should not own or duplicate credentials schema evolution; the persistence layer already has strict parsing and completeness validation.
- Decision: remove boot-local retired-field sentinel deserializers.
- Reason: old credential compatibility belongs nowhere in boot; `Credentials` already rejects unknown/retired fields through the owning schema.
