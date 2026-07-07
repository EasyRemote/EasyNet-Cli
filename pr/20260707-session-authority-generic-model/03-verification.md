# Verification

- `bash tools/scripts/check-sdk-conformance-reports.sh --self-test`
- `(cd sdk/go && go test ./...)`
- `PYTHONPATH=sdk/python python3 -m pytest sdk/python/tests/test_authority.py sdk/python/tests/test_cabi.py sdk/python/tests/test_conformance.py`
- `cargo test -q authority_metadata`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `bash tools/scripts/check-sdk-ura-naming.sh`
- `git diff --check`

