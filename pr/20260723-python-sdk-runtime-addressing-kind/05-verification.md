# Verification

## Checks

- `PYTHONPATH="$PWD/sdk/python:$PWD/../EasyNet-Axon/sdk/python" python -m pytest sdk/python/tests/test_axon_addressing.py sdk/python/tests/test_conformance_gates.py`
- `cargo fmt --check`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-architecture-convergence.sh`
- `git diff --check`
- `codegraph index .`
- `codegraph query _runtime_ura_kind --limit 40`
- `codegraph query _product_ura_kind --limit 40`
- `rg -n "_product_ura_kind|_product_ability_owner_kind" sdk/python/easynet_sdk/axon_addressing.py`

## Results

- `PYTHONPATH="$PWD/sdk/python:$PWD/../EasyNet-Axon/sdk/python" python -m pytest sdk/python/tests/test_axon_addressing.py sdk/python/tests/test_conformance_gates.py` passed: 14 tests.
- `cargo fmt --check` passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` passed.
- `bash tools/scripts/check-architecture-convergence.sh` passed.
- `git diff --check` passed.
- `codegraph index .` passed: 1,018 files indexed.
- `codegraph query _runtime_ura_kind --limit 40` found the new runtime helper.
- `codegraph query _product_ura_kind --limit 40` returned no SDK helper references.
- `rg -n "_product_ura_kind|_product_ability_owner_kind" sdk/python/easynet_sdk/axon_addressing.py` returned no matches.

Note: an initial pytest invocation without `PYTHONPATH` failed at import-time
because the local SDK package was not on the Python module path. The corrected
workspace-local invocation above is the behavioral verification result.
