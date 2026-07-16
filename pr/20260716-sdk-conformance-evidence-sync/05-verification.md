# Verification

Executed checks:

1. `bash tools/scripts/check-sdk-conformance-reports.sh --self-test` - passed.
2. `bash tools/scripts/check-sdk-canonical-public-api.sh` - passed.
3. `bash tools/scripts/check-sdk-parity-matrix.sh --self-test` - passed.
4. `bash tools/scripts/check-sdk-conformance-reports.sh` - passed after
   clearing stale generated conformance build artifacts from `target/`.
5. `EASYNET_SDK_PARITY_RESULTS_DIR=target/sdk-conformance-live-results EASYNET_SDK_PARITY_ALLOW_SNAPSHOT_RESULTS=1 bash tools/scripts/check-sdk-parity-matrix.sh` - passed.
6. `git diff --check` - passed.

Initial live parity failed with
`evidence_hash_mismatch:rust:bidi/close_send_not_cancel`; after synchronizing
runner evidence hashes and regenerating fresh live results, the parity matrix
accepted the current source snapshot.
