# Verification

Planned checks:

```sh
cd sdk/python && python -m pytest tests/test_publication.py tests/test_identity.py
cd sdk/python && python -m pytest tests/test_conformance.py -k publication
cd sdk/go && go test ./...
git diff --check
if rg -n 'record\.owner_ura\.(partition|split|rstrip|endswith)|owner_ura\.(partition|split|rstrip|endswith)' sdk/python/easynet_sdk/publication.py; then exit 1; else echo 'no Publication owner_ura text parsing remains'; fi
```

Actual checks:

```sh
cd sdk/python && python -m pytest tests/test_publication.py tests/test_identity.py
# 37 passed in 0.17s

cd sdk/python && python -m pytest tests/test_conformance.py -k publication
# 1 passed, 22 deselected in 0.17s

cd sdk/python && python -m pytest tests/test_conformance.py -k 'memc or publication'
# 4 passed, 19 deselected in 0.13s

cd sdk/python && python -m pytest
# 479 passed in 3.91s

cd sdk/go && go test ./...
# ok  	easynet.run/cli/sdk/go	(cached)

git diff --check
# pass

if rg -n 'record\.owner_ura\.(partition|split|rstrip|endswith)|owner_ura\.(partition|split|rstrip|endswith)' sdk/python/easynet_sdk/publication.py; then exit 1; else echo 'no Publication owner_ura text parsing remains'; fi
# no Publication owner_ura text parsing remains
```

Delta:

- `PublicationCatalogFacade.list_user()` now matches user-owned owner URAs
  through Identity projection components.
- Owner projections that lack `user_id` no longer match by textual URI shape.
- Python MEMC profile-ownership audit now accounts for
  `descriptor_bound_resource_subject_ura`.
