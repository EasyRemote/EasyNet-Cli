# Verification

Executed commands:

```sh
go test ./...
PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_identity.py
python -m py_compile sdk/python/easynet_sdk/identity.py sdk/python/easynet_sdk/__init__.py sdk/python/tests/test_identity.py
git diff --check
```

Results:

- Go SDK tests passed.
- Python identity tests passed.
- Python compile check passed.
- `git diff --check` passed.
- Go MEMC conformance now assigns `IdentityClient.ResourceURA` to
  `directory_identity.identity.resource_ura`.
- Full cutover scanner still fails on unrelated remaining backend boundary
  violations; backend-specific delta is recorded in the backend plan pack.
