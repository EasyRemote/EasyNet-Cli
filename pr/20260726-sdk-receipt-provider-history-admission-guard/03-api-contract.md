# API Contract

No public method signatures change.

## Behavior

`RuntimeReceiptProvider.List` / `RuntimeReceiptProvider.list` now reject invalid receipt-history call context before descriptor resolution:

- missing authority
- all-zero subject
- non runtime-state read subject
- caller/callee filter widening
- session/delegation authority mismatch

The rejection uses existing SDK error codes and stages.
