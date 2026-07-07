# Verification

Status: Passed.

Commands run:

```sh
cd sdk/go && go test ./...
PYTHONPATH=sdk/python python3 -m pytest sdk/python/tests/test_events.py sdk/python/tests/test_conformance.py sdk/python/tests/test_cabi.py -q
node --test sdk/node/test/runtime-core.test.mjs
bash tools/scripts/check-ffi-abi-v4-header.sh --self-test
bash tools/scripts/check-ffi-abi-v4-header.sh
TMPDIR=/tmp bash tools/scripts/check-sdk-scaffold.sh
bash tools/scripts/check-sdk-completion-audit.sh
git diff --check
```

Results:

- Go SDK tests passed.
- Python Events/conformance/C ABI tests passed: 97 passed.
- Node runtime core tests passed: 41 passed.
- SDK scaffold passed with negative checks for retired Events subscription aliases.
- FFI ABI v4 header self-test and real gate passed.
- SDK completion audit passed, including product smokes and Python/Go live daemon smokes.
- Downstream backend `go test ./...` passed after migrating the SDK event
  caller to stream-specific event query DTOs.
- Whitespace check passed.
