# Verification

Executed checks:

```sh
cargo test identity_contract::tests::project_signer_handle
cargo test ffi::identity::tests::identity_project_signer_handle_uses_daemon_inventory_output
cd sdk/go && go test ./...
cd sdk/python && python -m pytest tests/test_identity.py tests/test_signing.py tests/test_runtime.py
cd sdk/python && python -m pytest
tools/scripts/check-sdk-parity-matrix.sh
git diff --check
```

Results:

- `cargo test identity_contract::tests::project_signer_handle` passed: 3 tests.
- `cargo test ffi::identity::tests::identity_project_signer_handle_uses_daemon_inventory_output` passed: 1 test.
- `cd sdk/go && go test ./...` passed.
- `cd sdk/python && python -m pytest tests/test_identity.py tests/test_signing.py tests/test_runtime.py` passed: 53 tests.
- `cd sdk/python && python -m pytest` passed: 482 tests.
- `tools/scripts/check-sdk-parity-matrix.sh` passed.
- `git diff --check` passed.
