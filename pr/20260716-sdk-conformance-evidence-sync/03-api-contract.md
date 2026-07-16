# API Contract

This slice changes no public SDK API and no daemon wire contract.

The conformance contract is:

- Input: runner-owned action-adapter reports plus live result JSON files.
- Output: validated parity matrix acceptance or a typed failure code.
- Error behavior: stale evidence continues to fail with
  `evidence_hash_mismatch:<language>:<case_id>`.

Tenant and authority semantics are unchanged because this is proof metadata
only.
