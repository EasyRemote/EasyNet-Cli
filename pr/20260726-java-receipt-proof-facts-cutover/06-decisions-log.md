# Decisions Log

## 2026-07-26

- Treat Java proof-fact validation as part of the shared canonical runtime model, not a Java-specific DTO concern.
- Move Java proof-fact validation before summary projection so omitted mandatory fields cannot be reclassified as generic summary parsing failures.
- Use `RECEIPT_PROOF_FACTS_MISSING` only for absent required proof-fact keys; malformed or noncanonical present values remain `INVALID_ARGUMENT`.
