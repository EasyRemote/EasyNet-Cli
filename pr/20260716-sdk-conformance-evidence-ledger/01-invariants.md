## Invariants

1. Source test files remain the owner of executable evidence.
2. Adapter reports are an auditable ledger over current source hashes; they must
   not encode old compatibility windows.
3. A report hash mismatch is fail-closed evidence drift, not a runtime fallback.
4. This slice must not modify runtime behavior, SDK public APIs, or product
   consumers.
