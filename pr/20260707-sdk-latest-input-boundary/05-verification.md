# Verification

Verified:

- `go test ./...` from `sdk/go`: passed.
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_import_boundary.py sdk/python/tests/test_compatibility.py sdk/python/tests/test_admin.py sdk/python/tests/test_daemon.py sdk/python/tests/test_environment.py sdk/python/tests/test_cabi.py sdk/python/tests/test_conformance.py -q`: 140 passed.
- `python -m py_compile` over changed Python SDK/test modules: passed.
- `rg "\bURI\b|\bUri\b|\buri\b" sdk/go sdk/python/easynet_sdk src/ffi src/protocol include`: no matches.
- Legacy alias scan over Go/Python SDK surfaces found no short request type aliases or old public compatibility wrapper methods.
- `go test ./...` from `sdk/go`: passed after Runtime Core prepare-option convergence.
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_runtime.py sdk/python/tests/test_transport.py sdk/python/tests/test_ability_invocation.py sdk/python/tests/test_cabi.py sdk/python/tests/test_conformance.py -q`: 151 passed.
- `python -m py_compile sdk/python/easynet_sdk/runtime.py sdk/python/easynet_sdk/transport.py sdk/python/tests/test_ability_invocation.py sdk/python/tests/test_transport.py`: passed.
- `rg -n "require_user_sig|ResolveDescriptor|FillNonce|RequireUserSig|resolve_descriptor|fill_nonce" sdk/go sdk/python -g'*.go' -g'*.py'`: no matches.
- `rg -n "\bURI\b|\bUri\b|\buri\b" sdk/go sdk/python/easynet_sdk src/ffi src/protocol include docs/spec/daemon-sdk-requirements-v1.md`: no matches.

Not run:

- Full Rust/Cargo suite. This iteration changed Go/Python SDK facades and Python C ABI adapter method names, not Rust code.
