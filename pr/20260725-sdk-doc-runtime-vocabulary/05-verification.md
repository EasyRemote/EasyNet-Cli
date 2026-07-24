# Verification

## Vocabulary audit

- `rg -n "Daemon SDK|daemon-provided|daemon transport|daemon/runtime lifecycle|daemon-managed|daemon key-service|daemon-backed|OpenAI-compatibility|Directory/Identity projections|Host Binding|Admin/Gateway" sdk/go/README.md sdk/go/doc.go sdk/python/README.md sdk/node/README.md sdk/java/README.md sdk/swift/README.md`

Result: no matches.

## Gates

- `bash tools/scripts/check-sdk-product-neutrality.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `cd sdk/go && go test . -run 'TestConformanceSDKProductNeutrality|TestConformanceCanonicalPublicAPI'`

Result: all passed.
