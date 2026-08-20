# Verification

## Passed

- `python3 sdk/conformance/refresh_adapter_report_evidence.py --self-test`
- `python3 sdk/conformance/refresh_adapter_report_evidence.py --check`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `bash tests/scripts/test_check_sdk_scaffold.sh`
- `bash tools/scripts/check-sdk-canonical-public-api.sh`
- `bash tools/scripts/check-sdk-conformance-reports.sh --self-test`
- `bash tools/scripts/check-architecture-convergence.sh`
- `rm -rf sdk/conformance/__pycache__ && bash tools/scripts/check-project-structure-v1.sh`
- `git diff --check`

## Interrupted

- `SDK_CONFORMANCE_LANGUAGES=rust,c_abi,go,python bash tools/scripts/check-sdk-conformance-reports.sh`

The focused live report slice reached Rust report execution and then failed because the machine reported `No space left on device` while the shell was creating temporary here-doc files. The failure occurred before a conformance mismatch could be reported. The bounded self-test, evidence refresh self-test/currentness check, scaffold, canonical API, architecture, and project-structure gates passed.
