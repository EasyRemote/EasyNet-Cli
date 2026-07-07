# Verification

## Commands

- `cargo test ffi::features --lib` - passed.
- `cd sdk/go && go test ./...` - passed.
- `PYTHONPATH=sdk/python python3 -m pytest sdk/python/tests/test_client.py sdk/python/tests/test_cabi.py sdk/python/tests/test_conformance.py -q` - passed, 92 tests.
- `TMPDIR=/tmp bash tools/scripts/check-sdk-scaffold.sh` - passed.
- `bash tools/scripts/check-sdk-completion-audit.sh` - passed, including EasyRemote/backend product smokes and Python/Go live daemon smokes.
- `git diff --check` - passed before commit.

## Evidence

Feature discovery now reports only SPEC capability states in its `profiles`
object. Detailed provider facts remain in `symbols`, conformance cases, and the
SDK parity matrix.

The shared feature-discovery schema now restricts every profile status to:

- `unsupported`
- `seam`
- `provider-backed`
- `cutover-ready`

The active dispatch feature symbol is now `invocation_dispatch_v4`.
