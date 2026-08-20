# Verification

Planned checks:

| Check | Purpose | Result |
| --- | --- | --- |
| `cargo test --lib --features axon-pb daemon::admin_gateway_contract::tests::` | Shared Admin/Gateway DTO semantics | Pending |
| `cargo test --lib --features axon-pb ffi::admin_gateway::tests::` | C ABI pointer/handle/carrier/projection behavior | Pending |
| `cargo test --lib --features axon-pb ffi::` | Wider FFI regression | Pending |
| `cargo build --lib --features axon-pb` | Library compile with production feature | Pending |
| `EASYNET_FFI_REQUIRE_DYLIB=1 bash tools/scripts/check-ffi-abi-v4-header.sh` | ABI/header/export guard | Pending |
| `bash tests/scripts/test_check_ffi_abi_v4_header.sh` | ABI guard self-test | Pending |
| `bash tools/scripts/check-sdk-scaffold.sh` | SDK schema/scaffold consistency | Pending |
| `cargo fmt --check` | Rust formatting | Pending |
| `git diff --check` | Working tree whitespace check | Pending |
| `git diff --cached --check` | Staged whitespace check | Pending |

Completed 2026-07-05 Go C ABI gateway-status parity checks:

- `go test -tags 'easynet_cabi' -run 'TestCABIAdminTransport' .`
- `go test -tags 'easynet_cabi' ./...`
- `go test ./...`
- `cargo test admin_gateway --lib`
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_admin.py sdk/python/tests/test_cabi.py -q`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `cargo fmt --check`
- `git diff --check`
- Address-terminology scan on touched Go/Admin plan files: no old address
  spelling matches.
