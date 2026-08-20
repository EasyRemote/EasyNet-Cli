# Python SDK Product Alias Boundary Plan

## Objective

Remove the EasyRemote-specific alias facade from the Python SDK package root and
restore the daemon SDK as a product-neutral runtime model.

The Python SDK may be consumed by EasyRemote, but it must not define
EasyRemote-named public abstractions inside `easynet_sdk`. Product aliases
belong in the EasyRemote repository as wrappers over product-neutral SDK
objects.

## Boundary Proof

- SDK-owned classes remain available under neutral names such as
  `DaemonStartProjection`, `MissionPlan`, `AgentLifecycleAdapter`,
  `GatewayLifecycleFacade`, `PublicationCatalogFacade`, `LocalReceiptSummary`,
  `StreamValueAdapter`, and `BidiSessionAdapter`.
- EasyRemote compatibility names are not exported from `easynet_sdk.__all__`.
- No EasyRemote-specific module is required for importing the Python SDK package
  root from a clean checkout.
- External EasyRemote cutover tests may still model EasyRemote as a consumer;
  the SDK itself does not expose EasyRemote as an abstraction layer.

## Invariants

- The SPEC remains unchanged.
- Product-specific names do not become SDK public API.
- Product-neutral SDK profile objects are not renamed or removed.
- The cleanup must repair clean-checkout importability.

## Implementation Steps

1. Remove EasyRemote alias imports and exports from `easynet_sdk.__init__`.
2. Delete the untracked EasyRemote alias module and alias test residue.
3. Add a package-root neutrality regression test.
4. Run focused import tests plus full Python verification.

## Verification Plan

- `uv run python -m unittest tests.test_import_boundary`
- `uv run python -m unittest discover tests -p 'test_transport.py'`
- `uv run python -m unittest discover tests`
- `git diff --check`
- `git diff -- docs/spec/daemon-sdk-requirements-v1.md`

## Verification Result

- PASS: `uv run python -m unittest tests.test_import_boundary`
- PASS: `uv run python -m unittest discover tests -p 'test_transport.py'`
- PASS: `uv run python -m unittest discover tests`
- PASS: `git diff --check`
- PASS: `git diff -- docs/spec/daemon-sdk-requirements-v1.md` produced no diff.
