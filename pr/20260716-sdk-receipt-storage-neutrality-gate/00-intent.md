# Intent

Add regression guards for the receipt ledger-source boundary.

The SDK receipt model now exposes ledger authority by URA only. This slice keeps
that boundary executable by removing Go test fixtures that still depended on a
daemon-local ledger path and by making the SDK product-neutrality gate reject
future `LedgerPath` or `ledger_path` reintroduction.
