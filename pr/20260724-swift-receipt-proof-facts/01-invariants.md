# Invariants

## Receipt topology

- A terminal invocation result must contain a canonical `terminal_receipt`.
- `receipt` and `receipt_ref` are not terminal receipt aliases.
- Receipt lifecycle state and receipt type must match.
- Receipt proof facts must include authority proof, descriptor facts, hash facts, and parent receipt facts.

## SDK model

- Swift implements the same canonical runtime model as Go, Python, Java, and Node.
- No product-specific lifecycle, receipt, or directory semantics are introduced.
- URA terminology is required.
