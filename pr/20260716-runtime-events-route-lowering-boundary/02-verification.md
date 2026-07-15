# Verification

Planned checks:

- `go test ./runtimeevents` from `sdk/go`
- `tools/scripts/check-sdk-product-neutrality.sh --self-test`
- `tools/scripts/check-sdk-product-neutrality.sh`
- `python sdk/conformance/sdk_concepts.py --validate-actual`
- `python sdk/conformance/sdk_concepts.py --validate-schema`
- `tools/scripts/check-architecture-convergence.sh`
- `git diff --check`
