# Verification

## Commands

- `bash tools/scripts/check-java-sdk-seam.sh`
- `bash tools/scripts/check-swift-sdk-seam.sh`
- `bash tools/scripts/check-sdk-conformance-reports.sh`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `bash tools/scripts/check-sdk-ura-naming.sh`
- `bash tools/scripts/check-sdk-package-metadata.sh`
- `git diff --check`

## Evidence

- `bash tools/scripts/check-java-sdk-seam.sh`: passed.
- `bash tools/scripts/check-swift-sdk-seam.sh`: passed.
- `bash tools/scripts/check-sdk-conformance-reports.sh`: passed.
- `bash tools/scripts/check-sdk-scaffold.sh`: passed.
- `bash tools/scripts/check-sdk-ura-naming.sh`: passed.
- `bash tools/scripts/check-sdk-package-metadata.sh`: passed.
- `git diff --check`: passed.

Java and Swift Receipt seams now execute shared receipt fetch carrier and
summary/causal-ref guardrail tests. Summary projection remains non-cryptographic
unless a provider-backed Axon verifier supplies explicit evidence.
