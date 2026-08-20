# SDK Gate Bytecode Cache Evidence

## Source Exploration

- `tools/scripts/check-sdk-canonical-public-api.sh` invokes
  `sdk/conformance/rebuild_public_api_model.py`,
  `sdk/conformance/sdk_matrix.py`, `public_api_inventory.py`, and
  `sdk_concepts.py` without suppressing Python bytecode emission.
- `tools/scripts/check-sdk-parity-matrix.sh` invokes
  `sdk/conformance/sdk_matrix.py` without suppressing Python bytecode
  emission.
- `tools/scripts/check-sdk-conformance-reports.sh` invokes
  `sdk/conformance/refresh_adapter_report_evidence.py` and Python helpers;
  report verification should not mutate source directories.
- After `check-sdk-canonical-public-api.sh`, `check-project-structure-v1.sh`
  failed with:
  `unexpected directory under sdk/conformance: __pycache__`.

## Decision

Set `PYTHONDONTWRITEBYTECODE=1` in the SDK gate launchers before any Python
interpreter starts. This centralizes non-mutating behavior in the process
boundary instead of relying on developers to delete cache directories after
verification.
