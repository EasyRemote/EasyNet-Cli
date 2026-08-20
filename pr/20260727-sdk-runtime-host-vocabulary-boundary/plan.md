# SDK runtime-host vocabulary boundary

## Goal

Remove production SDK daemon/product vocabulary from canonical runtime-host lifecycle and transport diagnostics. The SDK must describe generic runtime concepts; EasyNet daemon details belong in provider adapters or downstream products.

## Root abstraction problem

The canonical SDK lifecycle model has already converged around `RuntimeHost`, `RuntimeLifecycleTransport`, and `RuntimeTransport`, but several production comments/diagnostics still say "daemon". That leaks provider/product ownership back into the SDK mental model and makes operator failures look like daemon lifecycle failures instead of runtime-host lifecycle failures.

## Invariants

1. The SDK root package exposes runtime-host lifecycle concepts, not daemon lifecycle concepts.
2. Provider adapters may translate provider wire fields, but generic client/runtime/connection errors must stay provider-neutral.
3. Public behavior and API shapes remain compatible; this slice changes diagnostics, comments, and guard coverage only.
4. No compatibility aliases or fallback paths are introduced.

## Planned changes

1. Replace production SDK root daemon wording in Go/Python generic runtime files with runtime-host/runtime-transport wording.
2. Add a focused architecture gate that rejects daemon lifecycle vocabulary in generic SDK production sources while allowing provider adapters and tests.
3. Run focused SDK tests and the canonical runtime convergence gate.

## Verification

- `bash tools/scripts/check-sdk-runtime-host-vocabulary-boundary.sh` — passed.
- `bash tests/scripts/test_check_sdk_runtime_host_vocabulary_boundary.sh` — passed.
- `bash tools/scripts/check-sdk-product-neutrality.sh` — passed after regenerating SDK conformance inventory from current source.
- `bash tools/scripts/check-sdk-parity-matrix.sh --self-test` — passed.
- `bash tools/scripts/check-sdk-canonical-public-api.sh` — passed.
- `go test ./...` from `sdk/go` — passed.
- `python -m pytest sdk/python/tests/test_client.py sdk/python/tests/test_transport.py sdk/python/tests/test_runtime_admin.py sdk/python/tests/test_conformance_gates.py` — passed.
- `bash -n tools/scripts/check-sdk-runtime-host-vocabulary-boundary.sh tests/scripts/test_check_sdk_runtime_host_vocabulary_boundary.sh tools/scripts/check-canonical-runtime-convergence-v2.sh` — passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — passed.
