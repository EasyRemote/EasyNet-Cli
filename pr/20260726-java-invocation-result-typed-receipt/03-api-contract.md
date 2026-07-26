# API Contract

## Existing contract preserved

- `new InvocationResult(boolean, InvocationTerminalState, String, SDKError, Map<String,Object>)`.
- `terminalReceipt()` returns the validated immutable raw receipt projection.

## New canonical accessor

- `runtimeReceipt()` returns a `RuntimeReceipt` aggregate validated from the same terminal receipt projection.

## Errors

- Missing or malformed proof facts continue to raise `SDKError` with `ErrorCode.RECEIPT_PROOF_FACTS_MISSING` or `ErrorCode.INVALID_ARGUMENT`.
- State/ok mismatches raise `SDKError.validation`.

## Tenant/security rules

This slice does not alter tenant routing. It tightens local SDK receipt interpretation so a product cannot accidentally treat unvalidated JSON as a terminal proof.
