# Verification

## Commands

- `bash tools/scripts/check-sdk-package-metadata.sh --self-test`
- `bash tools/scripts/check-sdk-package-metadata.sh`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `bash tools/scripts/check-sdk-cutover-readiness.sh`
- `git diff --check`

## Expected Evidence

- Self-test fixtures reject manifest drift.
- Repository manifests validate with the current capability-state labels.
- Scaffold and aggregate readiness include the package metadata gate.
