# Verification

Completed checks:

- `(cd sdk/go && go test -count=1 ./...)`
- `(cd sdk/go && go test -count=1 -tags easynet_cabi ./...)`
- `cargo fmt --check`
- `cargo test publication_`
- `PYTHONPATH=sdk/python:sdk/python/tests python -m pytest sdk/python/tests/test_publication.py sdk/python/tests/test_cabi.py -q`
- `ruff check sdk/python`
- `bash tools/scripts/check-sdk-scaffold.sh`
