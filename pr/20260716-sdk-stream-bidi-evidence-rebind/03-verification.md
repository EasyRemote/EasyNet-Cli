# Verification

Planned:

- `python3 sdk/conformance/rebuild_public_api_model.py`
- `python3 sdk/conformance/sdk_matrix.py --validate`
- `bash tools/scripts/check-sdk-canonical-public-api.sh`
- `bash tools/scripts/check-sdk-canonical-public-api.sh --self-test`
- `bash tools/scripts/check-sdk-parity-matrix.sh`
- `bash tools/scripts/check-sdk-completion-audit.sh --matrix-only`
- `bash tools/scripts/check-sdk-conformance-reports.sh`
- `git diff --check -- sdk/conformance pr/20260716-sdk-stream-bidi-evidence-rebind`

Completed:

- PASS: `codegraph status .`
- PASS: `codegraph node sdk/go/bidi_test.go`
- PASS: `codegraph node sdk/python/tests/test_bidi.py`
- PASS: `codegraph explore "stream bidi cancel conformance action adapter evidence report sdk parity matrix"`
- PASS: `python3 sdk/conformance/rebuild_public_api_model.py --write`
- PASS: generated-output compare showed `canonical-public-api.json` and
  `sdk-parity-matrix.json` match current generator output.
- PASS: `bash -n tools/scripts/check-sdk-canonical-public-api.sh`
- PASS: `bash tools/scripts/check-sdk-canonical-public-api.sh --self-test`
- PASS: `bash tools/scripts/check-sdk-canonical-public-api.sh`
- PASS: `SDK_CONFORMANCE_LANGUAGES=rust,c_abi,go,python SDK_CONFORMANCE_RESULT_DIR=target/sdk-conformance-evidence-rebind-results bash tools/scripts/check-sdk-conformance-reports.sh`
- PASS: `python3 sdk/conformance/sdk_matrix.py --validate-slice rust c_abi go python --results-dir target/sdk-conformance-evidence-rebind-results --allow-snapshot-results`
- PASS: `bash tools/scripts/check-sdk-completion-audit.sh --matrix-only`
- PASS: `bash tests/scripts/test_check_sdk_scaffold.sh`
- PASS: `bash tools/scripts/check-sdk-cutover-readiness.sh --self-test`
- PASS: `git diff --check -- sdk/conformance pr/20260716-sdk-stream-bidi-evidence-rebind`
- PASS: `git diff --check -- tools/scripts/check-sdk-canonical-public-api.sh pr/20260716-sdk-stream-bidi-evidence-rebind`

Observed outside this slice:

- Full `bash tools/scripts/check-sdk-conformance-reports.sh` validated Rust,
  C ABI, Go, Python, Node, and Java, then failed on a Swift run-nonce mismatch.
  The selected slice rerun used an isolated result directory and passed for all
  languages touched by the stale evidence finding.
