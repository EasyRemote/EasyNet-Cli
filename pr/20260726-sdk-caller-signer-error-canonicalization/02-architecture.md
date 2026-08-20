Architecture
============

Root abstraction
----------------

The SDK owns a canonical runtime error model. Transport adapters may receive
errors from C ABI, daemon gRPC, direct runtime providers, or future providers,
but all of them converge through the SDK error decoder.

Boundary
--------

- Go: `sdk/go/errors.go` owns `decodeRuntimeErrorJSON`.
- Python: `sdk/python/easynet_sdk/errors.py` owns `SDKError.from_json`.

Both language implementations apply the same semantic projection for
`CALLER_SIGNER_UNAVAILABLE`. This keeps product code and individual providers
from accumulating local redaction patches.

Ownership
---------

The daemon still owns signer custody and key-service storage. The SDK owns the
public error projection.
