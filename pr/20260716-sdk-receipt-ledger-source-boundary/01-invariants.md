# Invariants

- Go and Python expose the same `ReceiptLedgerSource` shape.
- `ReceiptLedgerSource` contains only `ledger_ura`.
- SDK receipt providers validate `ledger_ura` as a canonical URA before
  projecting history, get or trace results.
- The SDK must not expose daemon-local ledger paths as receipt authority.
- `canonical-public-api.json` and `sdk-parity-matrix.json` must be regenerated
  from actual public inventory after the public shape changes.
