# Verification

## Focused tests

- `sdk/python/.venv/bin/python -m pytest sdk/python/tests/test_import_boundary.py -q`
- `sdk/python/.venv/bin/python -m pytest sdk/python/tests/test_import_boundary.py sdk/python/tests/test_provider_ownership.py -q`
- `python3 -m py_compile sdk/python/easynet_sdk/consumer_boundary.py`

Result: all passed. The focused import-boundary suite reported `11 passed`;
the combined ownership/import run reported `19 passed`.

## Conformance attestation

- `python3 sdk/conformance/rebuild_public_api_model.py --write`

Result: canonical public API inventory was regenerated after the SDK source
change.

## Gates

- `bash tools/scripts/check-sdk-product-neutrality.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-backend-sdk-only-boundary.sh --self-test`

Result: all passed.

## Formatting and diff hygiene

- `cargo fmt --check`
- `git diff --check`
- `python3 -m py_compile sdk/python/easynet_sdk/consumer_boundary.py sdk/conformance/rebuild_public_api_model.py`

Result: all passed.
