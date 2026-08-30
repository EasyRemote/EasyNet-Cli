# Verification

- `/Users/macbook.silan.tech/.local/bin/codegraph status .`
  - index up to date; 44,133 nodes, 174,220 edges.
- `/Users/macbook.silan.tech/.local/bin/codegraph query AbilityPublication --limit 20`
  - confirms `src/daemon/ability/catalog/publication.rs::AbilityPublication`.
- `/Users/macbook.silan.tech/.local/bin/codegraph query RouteBinding --limit 20`
  - confirms `PublicationRouteBinding` and `AuthorityProofRouteBinding`.
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-sdk-product-neutrality.sh`
- `bash tools/scripts/check-ability-model-convergence.sh`
- `bash tools/scripts/check-transport-locator-terminology-boundary.sh`
- `bash tools/scripts/check-pending-dispatch-target-boundary.sh`
- `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet/backend && go test ./internal/catalog ./internal/runtimecontract ./internal/federation`
- `bash tools/scripts/check-sdk-canonical-public-api.sh`
- `bash tools/scripts/check-sdk-cutover-readiness.sh`

Observed result:

- First `check-sdk-cutover-readiness.sh` run failed only before the manifest
  refresh, at `SDK canonical public API`, because the committed manifest
  referenced Axon `e6e84299...` while current Axon was `028c3558...`.
- After `rebuild_public_api_model.py --write`, `check-sdk-canonical-public-api.sh`
  passed.
- After the refresh, full `check-sdk-cutover-readiness.sh` passed, including:
  SDK conformance reports, live parity matrix, generic FFI ABI v7 exact surface,
  SDK URA naming, canonical runtime convergence V2, product smokes,
  runtime-events live daemon E2E, standalone Hub PrincipalLifecycle E2E,
  Python SDK live smoke, and Go SDK live smoke.
