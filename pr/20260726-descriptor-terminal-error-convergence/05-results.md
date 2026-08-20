Results:

- Changed daemon route-negative status projection so `owner is not online`
  on `NXDOMAIN`/`NOROUTE` is availability (`Unavailable`) instead of descriptor
  absence (`NotFound`).
- Canonicalized remote forwarded failures to
  `DESCRIPTOR_OWNER_OFFLINE: descriptor owner is not online`.
- Added Go SDK canonicalization so legacy FFI JSON errors with
  `ABILITY_NOT_FOUND + ROUTE_NEGATIVE ... owner is not online` decode as
  `DESCRIPTOR_OWNER_OFFLINE`.
- Added Go direct-runtime gRPC canonicalization for both legacy `NOT_FOUND`
  and current `UNAVAILABLE` owner-offline payloads.
- Added Python SDK and Python direct-runtime parity for the same behavior.
- Rebuilt `sdk/conformance/canonical-public-api.json` and
  `sdk/conformance/sdk-parity-matrix.json` after SDK provider source changes.
- Strengthened SPEC v2 gates for daemon route status, remote failure projection,
  and Go/Python direct-runtime owner-offline projection.

Verification:

- `cargo test daemon::invocation::admission::target_gate::tests --lib` passed.
- `cargo test daemon::invocation::dispatch::remote_failure::tests --lib` passed.
- `cargo test invoke_unknown_ability_without_projection_returns_resolver_negative --lib` passed.
- `cargo test invoke_stream_unknown_function_returns_resolver_negative --lib` passed.
- `cargo test dispatch_local_rpc_selected_route_rejects_when_runtime_misses --lib` passed.
- `go test . -run 'Test(DecodeTransportErrorJSONCanonicalizesRouteOwnerOffline|DirectRuntimeGRPCErrorProjects.*Descriptor)'` in `sdk/go` passed.
- `PYTHONPATH="../EasyNet-Axon/sdk/python:sdk/python:sdk/python/tests" python -m pytest sdk/python/tests/test_errors.py -k 'route_owner_offline' sdk/python/tests/test_direct_runtime.py -k 'owner_offline' -q` passed.
- `python -m py_compile ...` for edited Python SDK/tests passed.
- `cargo fmt --check` passed.
- `git diff --check` passed.
- `bash tools/scripts/check-architecture-convergence.sh` passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test` passed.

Codegraph evidence:

- `codegraph sync .` reported the graph was up to date before implementation.
- `codegraph callers -p . caller_signer_unavailable_error` confirmed signer
  projection callers; `runtime_owner_unavailable` only had test coverage, so
  owner-offline work targeted route/SDK projection rather than signer custody.
