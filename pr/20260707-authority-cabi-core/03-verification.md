# Authority C ABI Core Verification

Required checks:

- Rust unit tests for signing material stability and metadata wire shape.
- Rust FFI unit tests for null, invalid JSON, and successful outputs.
- Header/scaffold check to ensure C ABI symbols remain documented.
- SDK parity update recording that concrete C ABI authority core exists while
  language concrete transport binding remains a separate cutover step.

## Results

- `cargo test authority --lib`: passed, 45 tests.
- `cargo test feature_catalog_matches_shared_conformance_fixture --lib`:
  passed, 1 test.
- `bash tools/scripts/check-sdk-scaffold.sh`: passed.
- `bash tools/scripts/check-sdk-parity-matrix.sh --self-test`: passed.
- `go test ./... -run 'TestAuthority|TestInvocationBuilder.*Authority|TestGoRuntimeCoreExecutesSharedAuthorityConformanceCase|TestFeature'`
  in `sdk/go`: passed.
- `go test -tags 'easynet_cabi cgo' ./... -run 'TestAuthority|TestFeature'`
  in `sdk/go`: passed.
- `go test ./... -run 'TestGoSDK|TestImport|TestAuthority'` in `sdk/go`:
  passed.
- `PYTHONPATH=sdk/python python3 -m unittest sdk/python/tests/test_authority.py sdk/python/tests/test_conformance.py -k authority`:
  passed, 10 tests.
- `PYTHONPATH=sdk/python:sdk/python/tests python3 -m unittest sdk/python/tests/test_cabi.py`:
  passed, 60 tests.
- `bash tools/scripts/check-sdk-cutover-readiness.sh`: still fails only at
  sibling EasyNet backend SDK-only boundary due raw Axon/generated protobuf and
  direct daemon transport imports.
