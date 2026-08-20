# Verification

1. `python3 sdk/conformance/sdk_concepts.py --validate-schema`
2. `python3 sdk/conformance/sdk_concepts.py --validate-actual`
3. `python3 sdk/conformance/sdk_concepts.py --self-test --tmp /tmp/easynet-sdk-concepts-selftest`
4. `python3 sdk/conformance/rebuild_public_api_model.py > /tmp/easynet-current-public-api.json`
5. `diff -u sdk/conformance/canonical-public-api.json /tmp/easynet-current-public-api.json`
6. `tools/scripts/check-sdk-product-neutrality.sh --self-test`
7. `tools/scripts/check-architecture-convergence.sh`
8. `git diff --check`
