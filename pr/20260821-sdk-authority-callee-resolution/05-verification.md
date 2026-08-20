# Verification

## Completed before commit `16f1e6abc`

- `cd sdk/go && go test ./...`
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python pytest -q sdk/python/tests`
- `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet/backend && go test ./...`
- `python3 sdk/conformance/rebuild_public_api_model.py --write`
- `python3 sdk/conformance/refresh_conformance_report_evidence.py --write`
- `python3 sdk/conformance/refresh_conformance_report_evidence.py --check`
- `bash tools/scripts/check-sdk-canonical-public-api.sh`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-sdk-cutover-readiness.sh`

## Completed after Python catalogue-read parity fix

- `python3 -m py_compile sdk/python/easynet_sdk/runtime.py sdk/python/easynet_sdk/runtime_ability.py`
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python pytest -q sdk/python/tests/test_runtime.py::RuntimeTests::test_descriptor_resolution_projects_catalogue_device_target_to_system_agent sdk/python/tests/test_runtime_ability.py::test_runtime_ability_catalogue_read_projects_device_target_to_system_agent sdk/python/tests/test_runtime_ability.py::test_runtime_ability_catalogue_read_resolves_descriptor_with_governance_read_subject sdk/python/tests/test_authorized_runtime_session.py::AuthorizedRuntimeSessionTests::test_runtime_client_descriptor_provider_uses_ability_descriptor_provider_for_catalogue_ability_ura`

## Required before next commit

- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python pytest -q sdk/python/tests`
- `python3 sdk/conformance/rebuild_public_api_model.py --write`
- `python3 sdk/conformance/refresh_conformance_report_evidence.py --write`
- `python3 sdk/conformance/refresh_conformance_report_evidence.py --check`
- `bash tools/scripts/check-sdk-canonical-public-api.sh`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-sdk-cutover-readiness.sh`
