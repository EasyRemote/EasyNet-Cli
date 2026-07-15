# Invariants

- Receipt tests must not require daemon-local ledger file paths to project
  receipt history, get or trace results.
- Product-neutral SDK sources, canonical public API inventory and parity matrix
  must not contain `LedgerPath` or `ledger_path`.
- The neutrality gate self-test includes a negative fixture that proves receipt
  storage path leaks are detected.
- This guard does not create a compatibility alias for the removed field.
