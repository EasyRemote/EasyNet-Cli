# Verification Matrix

| Check | Evidence |
| --- | --- |
| Go delegation mint facade | `go test ./... -run 'TestAuthority|TestInvocationBuilder.*Authority|TestGoRuntimeCoreExecutesSharedAuthorityConformanceCase'` |
| Go session authority mint facade | `go test ./... -run 'TestAuthority|TestInvocationBuilder.*Authority|TestGoRuntimeCoreExecutesSharedAuthorityConformanceCase'` |
| Go invalid request fails before transport | `go test ./... -run 'TestAuthority|TestInvocationBuilder.*Authority|TestGoRuntimeCoreExecutesSharedAuthorityConformanceCase'` |
| Python delegation/session mint facade | `PYTHONPATH=sdk/python python3 -m unittest sdk/python/tests/test_authority.py sdk/python/tests/test_conformance.py -k authority` |
| Python invalid request fails before transport | `PYTHONPATH=sdk/python python3 -m unittest sdk/python/tests/test_authority.py sdk/python/tests/test_conformance.py -k authority` |
| SDK scaffold | `bash tools/scripts/check-sdk-scaffold.sh` |
| SDK parity matrix | `bash tools/scripts/check-sdk-parity-matrix.sh --self-test` |
| SDK cutover readiness | `bash tools/scripts/check-sdk-cutover-readiness.sh` still fails only at sibling EasyNet backend SDK-only boundary. |

## Failure Paths

- Missing required authority binding field.
- Empty scopes/audiences.
- Expiry less than or equal to issued time.
- Transport returns malformed metadata.

## Recovery/Race

No runtime state machine is introduced in this slice. Client lifecycle is
guarded by the existing profile lifecycle close state.
