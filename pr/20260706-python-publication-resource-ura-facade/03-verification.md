# Verification

Planned checks:

```sh
cd sdk/python && python -m pytest tests/test_publication.py tests/test_identity.py
cd sdk/python && python -m pytest
cd sdk/go && go test ./...
git diff --check
if rg -n 'except TypeError|device\\.\\{node_id\\}|resource_ura\\(realm|owner_id' sdk/python/easynet_sdk/publication.py; then exit 1; else echo 'no legacy Publication resource_ura fallback remains'; fi
```

Actual checks:

```sh
cd sdk/python && python -m pytest tests/test_publication.py tests/test_identity.py
# 38 passed in 0.15s

cd sdk/python && python -m pytest
# 480 passed in 4.03s

cd sdk/go && go test ./...
# ok  	easynet.run/cli/sdk/go	(cached)

git diff --check
# pass

python -m py_compile sdk/python/easynet_sdk/publication.py sdk/python/tests/test_publication.py
# pass

if rg -n 'except TypeError|device\\.\\{node_id\\}|resource_ura\\(realm|owner_id' sdk/python/easynet_sdk/publication.py; then exit 1; else echo 'no legacy Publication resource_ura fallback remains'; fi
# no legacy Publication resource_ura fallback remains
```

Delta:

- `PublicationHostAdapter` now requires custom addressing facades to implement
  `resource_ura(owner_ura, path)`.
- The previous three-argument compatibility path and `device.<node_id>` owner
  id construction were removed.
- Tests now reject legacy resource-URA builder signatures before a daemon
  publication invocation is emitted.
