# Verification

- `PYTHONPATH=sdk/python python -m py_compile sdk/python/easynet_sdk/direct_runtime.py`
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_direct_runtime.py -q`
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_conformance.py -q`
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests -q`
- Forbidden-term scan across changed files passed for obsolete address terminology and legacy signing aliases.
- `git diff --check`

`bash tools/scripts/check-sdk-cutover-readiness.sh` still fails on the sibling backend SDK-only boundary in `/Users/macbook.silan.tech/Documents/GitHub/EasyNet/backend`, where raw Axon SDK imports, generated Axon protobuf packages, and direct daemon transport packages remain. This slice does not modify that repository.

## 2026-07-07 Follow-Up

- `PYTHONPATH=.:tests python -m unittest tests.test_direct_runtime` in
  `sdk/python`: passed.
- `PYTHONPATH=.:tests python -m unittest tests.test_cabi tests.test_direct_runtime`
  in `sdk/python`: passed.
- `bash tools/scripts/check-sdk-parity-matrix.sh`: passed.

Covered facts:

- Connector-level handle ownership closes the shared handle transport exactly
  once, after connector-created direct transports.
- Transport-level handle ownership closes the delegated handle transport exactly
  once, after the direct gRPC channel.
- Connector-created direct transports remain non-owning delegates even when a
  connector carries a shared handle transport.
- Prepare/submit/handle support is still advertised only when a handle transport
  is configured.
