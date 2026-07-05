# Python Direct Runtime Handle Transport

## Goal

Implement Python Direct runtime prepare, submit-signed, and invocation-handle operations through an explicit RuntimeTransport delegate while preserving direct Axon gRPC ownership for unary, stream, and bidi execution.

## Boundary Proof

- `DirectDaemonRuntimeTransport` continues to own direct daemon Axon gRPC Invoke, InvokeStream, and InvokeBidi calls.
- Prepare, submit-signed, await, cancel, handle-events, and free-handle remain daemon SDK Runtime Core semantics supplied by an explicit delegate.
- The C ABI Runtime transport already implements the delegate shape and remains the Rust-owned prepare/handle path.
- Python Direct runtime does not canonicalize signing material, sign prepared handles, or synthesize invocation handle state.
- No Axon protocol algorithm, keyring policy, or daemon handle registry is reimplemented in Python.

## Invariants

- Direct runtime handle operations fail closed without a configured delegate.
- Direct unary/stream/bidi behavior remains unchanged.
- Connector handshake reports prepare/submit support only when a delegate is configured.
- Delegate ownership is external; closing direct gRPC transport does not close the delegate.
- No retired address terminology is introduced in touched files.

## Verification

- `PYTHONPATH=sdk/python:sdk/python/tests python -m pytest sdk/python/tests/test_direct_runtime.py -q` - passed, 14 tests.
- `PYTHONPATH=sdk/python:sdk/python/tests python -m pytest sdk/python/tests -q` - passed, 440 tests.
- `python -m compileall -q sdk/python/easynet_sdk/direct_runtime.py sdk/python/tests/test_direct_runtime.py` - passed.
- `ruff check sdk/python` - passed.
- `bash tools/scripts/check-sdk-scaffold.sh` - passed.
- `git diff --check` - passed.
