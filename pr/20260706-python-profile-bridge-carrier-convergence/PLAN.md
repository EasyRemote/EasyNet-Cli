# Python Profile Bridge Carrier Convergence Plan

## Objective

Make the Python profile bridge satisfy the Admin + Gateway and Mission carrier
builder obligations from `daemon-sdk-requirements-v1.md`. The bridge must stop
reporting SDK-owned Invocation carrier construction as unsupported while keeping
product execution policy outside the SDK.

## Invariants

- The SPEC remains unchanged.
- Admin + Gateway and Mission carrier builders produce complete Invocation
  carriers through SDK-owned DTO/projection code.
- DescriptorRef construction is delegated through the SDK Directory + Identity
  facade and Axon-owned helpers. The profile bridge must not parse
  `easynet:///r/...`, derive ability owners, or append `@descriptor_version`
  locally.
- System ability execution remains delegated to the profile bridge dispatcher.
- The bridge does not introduce product decorators, direct lower-layer ABI
  exposure, or product transport ownership.
- Invalid carrier input is rejected with typed SDK errors at the profile stage.

## Implementation Steps

1. Audit Python Admin/Mission clients and profile bridge transport seams.
2. Add or reuse Python SDK carrier projection helpers for Admin + Gateway and
   Mission profile bridge methods.
3. Replace unsupported profile bridge builder stubs with complete Invocation
   carrier builders.
4. Route system DescriptorRef construction through an injectable
   `ProfileBridgeAddressing` facade so tests can prove delegation while
   production defaults to the SDK identity facade.
5. Add focused tests proving the profile bridge exposes the builder methods,
   delegates DescriptorRef construction, and rejects invalid carrier inputs
   through SDK typed errors.
6. Run Python tests, shared conformance runner, SPEC diff, whitespace, and
   forbidden lower-layer terminology checks before committing.

## Boundary Proof

The profile bridge is allowed to adapt host-application dispatcher execution
into daemon SDK profile DTOs. It is not allowed to make host applications
construct lower-layer Invocation carrier JSON themselves. Carrier construction
belongs to the Python Daemon SDK profile clients, matching the Go/C ABI/Rust
profile behavior. Dispatcher-backed execution methods continue to own only the
execution handoff.

The profile bridge also cannot become a second Axon addressing source. Its only
DescriptorRef dependency is the narrow `ProfileBridgeAddressing` protocol. The
default implementation calls `easynet_sdk.identity.owner_ability_descriptor_ref`,
which is the Python SDK facade over Axon-owned URA and descriptor-ref helpers.

## Verification Plan

- `uv run python -m unittest discover tests` in `sdk/python`
- focused Python profile bridge tests
- `cargo run --bin sdk-conformance-runner -- --language python --adapter-report
  sdk/conformance/runner/python-action-adapter-report.json --format jsonl`
- `cargo test --bin sdk-conformance-runner`
- `git diff --check`
- `git diff -- docs/spec/daemon-sdk-requirements-v1.md`
- search touched files for forbidden lower-layer transport and non-URA drift
