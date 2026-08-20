# Verification Plan

## Static Checks

1. `rg` direct Axon imports in SDK language roots.
2. Go SDK import-boundary test.
3. SDK scaffold, parity, URA naming, daemon latest input boundary, and daemon
   Invocation migration guards.
4. Backend SDK-only boundary and route-family coverage checks.

## Product Checks

1. EasyRemote product tests through SDK boundary.
2. EasyNet backend product tests through Go SDK boundary.
3. Python SDK live daemon smoke.
4. Go SDK live daemon smoke.

## Package Checks

1. `cd sdk/go && go test ./...`
2. `cd sdk/go && go test -tags easynet_direct_runtime ./...`
3. `cd sdk/go && CGO_ENABLED=1 go test -tags easynet_cabi ./...`
4. `cd sdk/python && PYTHONPATH=. uv run pytest -q`

## Audit Output

Record whether each remaining direct-Axon language SDK file is an allowed
facade bridge or an implementation leak. Any implementation leak must be fixed
before claiming completion.

## Results

Current allowed Go direct-Axon bridge files:

| File | Verdict |
| --- | --- |
| `sdk/go/ability_descriptor_axon.go` | Allowed bridge to Axon DescriptorRef parser. |
| `sdk/go/authority_axon.go` | Allowed bridge to Axon authority signing/materialization helpers. |
| `sdk/go/invocation_canonical.go` | Allowed bridge to Axon canonical Invocation bytes. |
| `sdk/go/invoke_remote.go` | Allowed bridge to Axon remote-invoke envelope helpers. |
| `sdk/go/ura.go` | Fixed in this slice: no longer owns URI grammar; keeps SDK DTOs and delegates parse/build semantics to Axon. |

Verification commands run:

```text
cd sdk/go && go test ./...
cd sdk/go && go test -tags easynet_direct_runtime ./...
cd sdk/go && CGO_ENABLED=1 go test -tags easynet_cabi ./...
cd sdk/python && PYTHONPATH=. uv run pytest -q
bash tools/scripts/check-sdk-cutover-readiness.sh
```

All commands passed. The aggregate readiness gate also passed EasyRemote
product tests, backend product tests, Python SDK live daemon smoke, and Go SDK
live daemon smoke.
