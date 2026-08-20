# Verification

Planned checks:

- `python sdk/conformance/public_api_inventory.py --self-test`
- `tools/scripts/check-sdk-canonical-public-api.sh --self-test`
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_import_boundary.py -q`
- `PYTHONPATH=sdk/python MYPYPATH=../EasyNet-Axon/sdk/python mypy --strict --follow-imports=skip sdk/python/easynet_sdk sdk/conformance/python_sdk_type_contract.py`
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_access_control.py sdk/python/tests/test_conformance_gates.py -q`
- `tools/scripts/check-architecture-convergence.sh`
