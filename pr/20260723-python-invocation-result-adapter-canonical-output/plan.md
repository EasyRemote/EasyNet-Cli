# Python invocation result adapter canonical output

## Goal

Remove the Python-only `InvocationResultAdapter` success wrapper that projects canonical runtime invocation results into legacy `result_*` fields and numeric terminal state codes.

The canonical SDK result model uses `output_content_type`, `output_base64`, `output_json`, `terminal_state`, and receipt facts. A Python facade that returns `result_content_type`, `result_base64`, `result_json`, `state`, and `sdk_runtime_result` preserves a second result model that has no Go parity.

## Boundary proof

- The adapter remains a runtime facade and preserves failure behavior: non-`ok` runtime results still raise `SDKError`.
- Successful adapter calls return the canonical runtime result dictionary produced by `RuntimeInvocationTransport`.
- Protobuf-generated fields named `result_content_type` are wire facts and are not in scope for this SDK facade projection.
- Removing the numeric terminal-state projection deletes a Python-only lifecycle fork; typed runtime state remains in `terminal_state`.

## Invariants

1. `InvocationResultAdapter.invoke()` returns canonical `output_*` fields on success.
2. `InvocationResultAdapter.invoke_signed()` returns the same canonical shape.
3. The adapter does not emit `result_content_type`, `result_base64`, `result_json`, `state`, or `sdk_runtime_result` wrapper fields.
4. Non-success runtime results still raise typed `SDKError`.
5. SPEC v2 gate rejects reintroduction of the wrapper.

## Verification plan

- Python transport focused tests.
- SPEC v2 gate.
- SDK product-neutrality, architecture convergence, public API gates.
- codegraph sync/status.

## Delta log

- Removed the Python-only success wrapper from `InvocationResultAdapter`.
- Preserved failure projection as typed `SDKError` while returning canonical runtime result dicts on success.
- Deleted numeric terminal-state code fallback helpers and unused output coercion helpers.
- Updated focused transport tests to assert canonical `output_*` fields and reject `result_*` / `sdk_runtime_result`.
- Added SPEC v2 structural and mutation coverage for the adapter projection.
- Verified focused Python transport tests, fmt, SPEC v2, SDK product-neutrality, architecture convergence, public API, and codegraph.
