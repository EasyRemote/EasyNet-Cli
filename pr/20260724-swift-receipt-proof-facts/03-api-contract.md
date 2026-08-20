# API Contract

## Public behavior

- `InvocationResult.terminalReceipt` remains a `[String: Any]` projection.
- Invalid terminal receipt shapes raise `SDKError.invalidArgument`.
- Retired `receipt` aliases are rejected.

## Canonical receipt fields

- `invocation_id`, `receipt_type`, `state`, `prev_receipt_hash_hex`, `self_hash_hex`, and `authority_proof` are mandatory.
- Hashes must be exactly 32-byte hex strings.
- Proof facts must include parent receipt topology and descriptor facts.
