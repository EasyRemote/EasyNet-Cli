# Verification

Results:

- `go test ./... -run 'AbilityDescriptor|Directory|Conformance|ImportBoundary'`: passed from `sdk/go`.
- `python -m pytest tests/test_publication.py -k 'AbilityDescriptorProjection'`: passed.
- `python -m pytest tests/test_publication.py tests/test_import_boundary.py -q`: passed.
- `bash tools/scripts/check-sdk-scaffold.sh`: passed.
- `bash tools/scripts/check-sdk-parity-matrix.sh --self-test`: passed.
- `git diff --check`: passed for EasyNet-Cli and backend.
