# Verification

## Passed

- `PYTHONPATH=sdk/python python3 -m pytest sdk/python/tests/test_invocation.py sdk/python/tests/test_runtime.py sdk/python/tests/test_signing.py`
- `PYTHONPATH=sdk/python python3 -m pytest sdk/python/tests`
- `cd sdk/go && go test ./...`
- `cd sdk/go && go test -tags=easynet_direct_runtime ./...`

## Known Remaining Gate Failure

- `tools/scripts/check-backend-sdk-only-boundary.sh /Users/macbook.silan.tech/Documents/GitHub/EasyNet/backend`

The backend boundary checker still reports direct `internal/daemon_grpc`, generated Axon protobuf package, and remaining service imports. This slice only tightens shared Runtime Core DTO validation.
