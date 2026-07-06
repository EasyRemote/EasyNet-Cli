# Verification

## Passed

- `PYTHONPATH=sdk/python python3 -m pytest sdk/python/tests/test_invocation.py sdk/python/tests/test_runtime.py sdk/python/tests/test_signing.py`
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_invocation.py sdk/python/tests/test_identity.py sdk/python/tests/test_environment.py sdk/python/tests/test_conformance.py -k "identity or invocation or descriptor_ref"`
- `PYTHONPATH=sdk/python python3 -m pytest sdk/python/tests`
- `cd sdk/go && go test ./...`
- `cd sdk/go && go test -tags=easynet_direct_runtime ./...`

## Known Remaining Gate Failure

- `tools/scripts/check-backend-sdk-only-boundary.sh /Users/macbook.silan.tech/Documents/GitHub/EasyNet/backend`

The backend boundary checker still reports direct `internal/daemon_grpc`, generated Axon protobuf package, and remaining service imports. This slice keeps Python descriptor-ref projection on the SDK Identity/Addressing facade and removes local Runtime Core grammar ownership.
