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

Java and Swift now prove local bidi send-side closure as an explicit state:
`closeSend` keeps receive active, `send` after `closeSend` returns typed
`CANCELLED`, and full `close` releases the local handle.
