# CLI Invocation URA Audit Plan

## Scope

Verify and update the CLI-side invocation audit path so every persisted,
queried, and returned identifier is a complete EasyNet URA
(`easynet:///r/<realm>/...`) and no user-facing invocation ledger field
uses the old URI vocabulary.

## Invariants

- Invocation records persist `invocation_ura`, `caller_ura`, `callee_ura`,
  `subject_ura`, and `ability_ura` from the envelope/URA builder boundary.
- Receipt anchors and trace DAG edges use `receipt_ura`, never
  `receipt_uri`.
- History abilities return `ledger_ura`, not `ledger_uri`, and never ask UI
  or backend code to reconstruct a resource address.
- A daemon-open ledger is read through the injected shared `Arc`; request
  handlers must not reopen the same redb file and trip the exclusive lock.

## Verification

- Target Axon Rust ledger unit tests for trace edge and receipt anchor field
  names.
- Target EasyNet CLI invocation history ability tests for shared-ledger reads
  and JSON response field names.
- Target CLI invocation group tests if available; otherwise run the relevant
  crate test filters.
