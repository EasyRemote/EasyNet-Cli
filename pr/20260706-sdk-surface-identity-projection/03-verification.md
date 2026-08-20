# Verification

Planned checks:

```sh
cd sdk/go && go test ./...
cd sdk/go && go test . -run 'TestSurfaceRuntimeTransport|TestGoSurfaceFacade'
git diff --check
rg -n 'fmt\.Sprintf\("easynet:///r/%s/(resource|agent)|surfaceRef = fmt\.Sprintf|ownerURA = fmt\.Sprintf' sdk/go/surface_runtime.go
```

Actual checks:

```sh
cd sdk/go && go test . -run 'TestSurfaceRuntimeTransport|TestGoSurfaceFacade'
# ok  	easynet.run/cli/sdk/go	0.749s

cd sdk/go && go test ./...
# ok  	easynet.run/cli/sdk/go	1.469s

cd sdk/python && python -m pytest tests/test_identity.py tests/test_surface.py
# 20 passed in 0.12s

git diff --check
# pass

if rg -n 'fmt\.Sprintf\("easynet:///r/%s/(resource|agent)|surfaceRef = fmt\.Sprintf|ownerURA = fmt\.Sprintf|realmFromURA|Realm:' sdk/go/surface_runtime.go; then exit 1; else echo 'no static Surface URA construction remains'; fi
# no static Surface URA construction remains
```

Delta:

- `SurfaceRuntimeTransport` now passes the request context and `IdentityClient`
  into Surface projection hints.
- Missing daemon `surface_ref` values are completed through
  `IdentityClient.ResourceURA(ctx, owner_ura, page_id)`.
- The previous Surface-local `realmFromURA` parser and static
  `easynet:///r/...` formatting path were removed.
- The focused Surface runtime test now covers a daemon row without
  `project_ura` and asserts `BuildURA(kind=resource)` was used.
