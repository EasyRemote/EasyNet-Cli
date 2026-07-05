# Verification

All checks were run from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`
unless noted.

- `gofmt -w sdk/go/cabi_admin.go sdk/go/cabi_admin_test.go`
- `cargo fmt`
- `cargo test admin_gateway_contract --lib`
- `cargo test admin_build_session --lib`
- `cargo test admin_project_device_session_result --lib`
- `go test -count=1 -tags easynet_cabi -run 'TestCABIAdminTransport' ./...`
  from `sdk/go`
- `go test -count=1 ./...` from `sdk/go`
- `go test -count=1 -tags easynet_cabi ./...` from `sdk/go`
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_cabi.py -k 'admin_device_session or admin_trust_mutations or admin_list_device_sessions or admin_revoke_device'`
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_cabi.py`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `cargo fmt --check`
- `git diff --check`
- Retired address terminology scan over touched files.
