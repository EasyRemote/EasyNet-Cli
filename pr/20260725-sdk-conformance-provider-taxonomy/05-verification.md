# Verification

## Inventory regeneration

- `python3 sdk/conformance/rebuild_public_api_model.py --write`

Result: regenerated `sdk/conformance/canonical-public-api.json`; Python root,
Java, and Swift distribution roots now use `distribution_facade`.

## Focused checks

- `sdk/python/.venv/bin/python -m pytest sdk/python/tests/test_provider_ownership.py -q`
- `python3 -m py_compile sdk/conformance/sdk_concepts.py sdk/conformance/rebuild_public_api_model.py sdk/conformance/sdk_public_surface_policy.py`
- `python3 sdk/conformance/sdk_concepts.py --manifest sdk/conformance/canonical-public-api.json --print-neutrality-roots | sort`
- `cd sdk/go && go test . -run 'TestConformanceSDKProductNeutrality|TestConformanceCanonicalPublicAPI'`

Result: all passed. Neutrality roots remain limited to Go/Python
provider-neutral core/provider-runtime roots.

## Gates

- `bash tools/scripts/check-sdk-product-neutrality.sh --self-test`
- `bash tools/scripts/check-sdk-product-neutrality.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`

Result: all passed.

## Formatting and diff hygiene

- `cargo fmt --check`
- `git diff --check`

Result: both passed.
