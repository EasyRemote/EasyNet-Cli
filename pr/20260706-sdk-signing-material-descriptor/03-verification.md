# Verification

Executed checks:

```sh
cd sdk/go && go test ./...
cd sdk/python && python -m pytest tests/test_signing.py tests/test_runtime.py
cd sdk/python && python -m pytest
git diff --check
if rg -n 'fallbackDescriptorRef|fallback_descriptor_ref|or fallback_descriptor_ref|material\\.descriptorRef = fallback' sdk/go/signing.go sdk/python/easynet_sdk/signing.py; then exit 1; else echo 'no signing material descriptor fallback remains'; fi
```

Results:

- `cd sdk/go && go test ./...` passed.
- `cd sdk/python && python -m pytest tests/test_signing.py tests/test_runtime.py` passed: 37 tests.
- `cd sdk/python && python -m pytest` passed: 482 tests.
- `git diff --check` passed.
- Fallback search returned no matches.
