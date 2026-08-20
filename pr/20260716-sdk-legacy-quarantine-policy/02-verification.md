# Verification

Planned checks:

- `python sdk/conformance/sdk_concepts.py --self-test --tmp /tmp/easynet-sdk-concepts-selftest`
- `python sdk/conformance/sdk_concepts.py --validate-schema`
- `python sdk/conformance/sdk_concepts.py --validate-actual`
- `python sdk/conformance/rebuild_public_api_model.py > /tmp/easynet-canonical-public-api.json && diff -u sdk/conformance/canonical-public-api.json /tmp/easynet-canonical-public-api.json`
- `tools/scripts/check-architecture-convergence.sh`
- `git diff --check`
