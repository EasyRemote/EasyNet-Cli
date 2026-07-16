# Verification

Passed:

```text
PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_runtime_ability.py -q
PYTHONPATH=sdk/python python -m pytest sdk/python/tests -q
bash tools/scripts/check-sdk-product-neutrality.sh --self-test
bash tools/scripts/check-sdk-product-neutrality.sh
python3 sdk/conformance/rebuild_public_api_model.py --write
bash tools/scripts/check-sdk-canonical-public-api.sh --self-test
bash tools/scripts/check-sdk-canonical-public-api.sh
bash tools/scripts/check-architecture-convergence.sh
bash tests/scripts/test_check_architecture_convergence.sh
```

The full suite passes 373 tests. The focused cases prove Python requests an
RPC descriptor for unary lowering, requests a stream descriptor for stream
lowering, and fails closed when no runtime descriptor resolver exists. R59
rejects a return to Addressing-owned descriptor minting.
