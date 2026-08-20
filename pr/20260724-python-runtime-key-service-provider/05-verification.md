# Verification

## Focused Python tests

- `sdk/python/.venv/bin/python -m pytest sdk/python/tests/test_managed_signing.py sdk/python/tests/test_runtime_identity.py sdk/python/tests/test_provider_ownership.py -q`

Result: `32 passed, 20 subtests passed`.

## Codegraph

- `codegraph sync .`
- `codegraph query "providers/easynet/keyring" --limit 20`
- `codegraph query "providers.runtime.keyring" --limit 40`
- `codegraph query "providers.runtime.key_service" --limit 40`

Result: retired `providers/easynet/keyring` had no results; runtime keyring/key-service symbols were present.

## Gates

- `bash tools/scripts/check-sdk-product-neutrality.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`

Result: all passed.

## Formatting and diff hygiene

- `cargo fmt --check`
- `git diff --check`

Result: both passed.

## Conformance attestation

- `python3 sdk/conformance/rebuild_public_api_model.py --write`

Result: canonical public API inventory and SDK parity matrix record Python runtime provider ownership for key-service/keyring helpers.
