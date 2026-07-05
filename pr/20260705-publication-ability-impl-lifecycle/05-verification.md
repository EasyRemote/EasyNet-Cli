# Verification

Completed checks:

- `cargo test publication_` passed.
- `PYTHONPATH=sdk/python:sdk/python/tests python -m pytest sdk/python/tests/test_publication.py sdk/python/tests/test_cabi.py -q` passed with 72 tests.
- `PYTHONPATH=sdk/python:sdk/python/tests python -m pytest sdk/python/tests -q` passed with 433 tests.
- `ruff check sdk/python` passed.
- `bash tools/scripts/check-sdk-scaffold.sh` passed.

Failure-path coverage:

- Missing complete tuple fields is rejected before runtime dispatch in C ABI tests.
- Non-Ability URA is rejected at the Rust/C ABI carrier boundary.
- Daemon runtime dispatch path is covered by Python RuntimePublicationTransport and CABI publication tests.
- Invalid projected daemon output kind is still guarded by `PublicationClient._expect_record`.
