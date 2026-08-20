# Verification

All checks were run from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`
unless noted.

- `gofmt -w sdk/go/cabi_mission.go sdk/go/cabi_mission_test.go`
- `cargo fmt`
- `python -m py_compile sdk/python/easynet_sdk/_cabi.py sdk/python/tests/test_cabi.py`
- `cargo test mission_contract --lib`
- `cargo test mission_build_events --lib`
- `cargo test mission_project_events --lib`
- `go test -count=1 -tags easynet_cabi -run 'TestCABIMissionTransport' ./...`
  from `sdk/go`
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_cabi.py -k 'mission_live_methods_use_carrier_invoke_and_projection'`
- `go test -count=1 ./...` from `sdk/go`
- `go test -count=1 -tags easynet_cabi ./...` from `sdk/go`
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_cabi.py`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `cargo fmt --check`
- `git diff --check`
- Retired address terminology scan over touched files.
