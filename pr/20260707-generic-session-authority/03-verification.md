# Verification

Required commands:

```text
cargo test authority_metadata
cargo test parse_and_verify_session_authority
go test ./sdk/go
python -m pytest sdk/python/tests/test_authority.py sdk/python/tests/test_conformance.py
npm test --prefix sdk/node
bash tools/scripts/check-sdk-ura-naming.sh
bash tools/scripts/check-sdk-scaffold.sh
bash tools/scripts/check-sdk-parity-matrix.sh
bash tools/scripts/check-sdk-conformance-reports.sh
git diff --check
```

Expected result: shared authority fixture/schema use generic session authority,
Go/Python/Node pass authority projection and mutual-exclusion tests, and no
retired session-authority input fields remain in public authority facades.

Observed result:

- `cargo test authority_metadata`: passed.
- `cargo test parse_and_verify_session_authority`: passed.
- `go test ./...` from `sdk/go`: passed.
- `PYTHONPATH=sdk/python python3 -m pytest sdk/python/tests/test_authority.py sdk/python/tests/test_conformance.py`:
  passed, 33 tests.
- `npm test --prefix sdk/node`: passed, 41 tests.
- `bash tools/scripts/check-sdk-ura-naming.sh`: passed.
- `bash tools/scripts/check-node-sdk-seam.sh`: passed.
- `bash tools/scripts/check-sdk-scaffold.sh`: passed.
- `bash tools/scripts/check-sdk-parity-matrix.sh`: passed.
- `bash tools/scripts/check-sdk-conformance-reports.sh`: passed.
- `git diff --check`: passed.
