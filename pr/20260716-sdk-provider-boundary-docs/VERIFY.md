# Verification

## Passed

- `python3 - <<'PY' ...` inventory check:
  - Go canonical inventory contains `RuntimeHost`.
  - Python canonical inventory contains `RuntimeHost`.
  - Non-canonical inventory contains `DaemonControl`.
  - Non-canonical inventory contains `RuntimeHostRole`.
  - Capability inventory count is 31.
- `bash tools/scripts/check-sdk-canonical-public-api.sh`
- `bash tools/scripts/check-sdk-parity-matrix.sh --self-test`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `python3 sdk/conformance/sdk_matrix.py --generate > /tmp/easynet-sdk-parity.generated.json && cmp -s /tmp/easynet-sdk-parity.generated.json sdk/conformance/sdk-parity-matrix.json`
- `bash tools/scripts/check-architecture-convergence.sh`
- `rg -n "\\bURI\\b|_uri\\b|\\buri\\b" docs/spec/daemon-sdk-requirements-v1.md sdk/SDK_INTERFACE_SPEC.md sdk/SDK_PARITY.md && exit 1 || echo 'URA naming docs ok'`
- `bash tools/scripts/check-sdk-conformance-reports.sh --self-test`
- `rm -rf sdk/conformance/__pycache__ && bash tools/scripts/check-project-structure-v1.sh`
- `git diff --check`

## Scope note

This slice is documentation/source-of-truth convergence. Existing Rust file diffs in the working tree are formatting-only changes from another in-flight slice and are intentionally not staged here.
