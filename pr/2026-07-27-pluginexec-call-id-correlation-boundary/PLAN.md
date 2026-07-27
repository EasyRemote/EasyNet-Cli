## Intent

Retire pluginexec sidecar helpers' empty `call_id` fallback so request/response correlation remains a canonical runtime fact across Python, Node, and Java helpers.

## Boundary invariant

- `call_id` is the sidecar request/response correlation handle for one runtime-admitted invocation.
- A sidecar helper may convert handler failures into protocol `error` frames only after a canonical invocation frame has been parsed and the `call_id` is known.
- Malformed request frames must fail closed instead of emitting uncorrelated `{"call_id":""}` error frames.

## Decision

Move handler exception framing behind successful `SidecarInvocation` parsing in Python, Node, and Java pluginexec helpers. Keep the public helper names and canonical request/result/error frame shapes unchanged.

## Verification target

- Python pluginexec tests.
- Node pluginexec tests.
- Java SDK seam test covering provider runtime pluginexec helpers.
- SDK public API/conformance gate because provider source attestations may change.
