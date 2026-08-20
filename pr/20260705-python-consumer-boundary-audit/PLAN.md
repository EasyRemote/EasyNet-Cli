# Python Consumer Boundary Audit Neutrality

## Objective

Remove EasyRemote-specific public naming from the Python SDK cutover audit helper while preserving the consumer boundary checks required by `docs/spec/daemon-sdk-requirements-v1.md`.

## Boundary Proof

- Ownership: raw lower-layer import, raw C ABI, raw transport, raw Invocation codec, receipt continuity, addressing, publication, admin, mission, and host-stream checks are generic SDK consumer boundary rules.
- Product boundary: EasyRemote remains an acceptance consumer in SPEC case IDs and tests, but the SDK must not export an EasyRemote-specific auditor type.
- Compatibility posture: old product-named public auditor classes/functions are removed rather than aliased so the SDK exposes one consumer boundary audit model.
- Runtime impact: the audit remains static; no daemon runtime, Invocation construction, or receipt semantics are changed.

## Implementation

- Rename `EasyRemoteCutoverAuditor` to `ConsumerBoundaryAuditor`.
- Rename `audit_easyremote_cutover` to `audit_consumer_boundary`.
- Update package exports, conformance tests, cutover audit tests, and consumer acceptance tests.
- Update SDK docs and parity notes to describe consumer boundary audit helpers.
- Tighten DescriptorRef parsing detection so `partition("@")` and
  `rpartition("@")` over `descriptor_ref` are rejected the same way as
  `split("@")` and `rsplit("@")`. Consumers must use SDK/Axon projection helpers
  instead of decomposing the descriptor grammar locally.

## Verification

- Python cutover audit tests.
- EasyRemote consumer acceptance tests.
- Python conformance tests touching cutover audit gates.
- Full Python SDK tests.
- Go SDK tests.
- SDK scaffold check.
- Formatting, diff, and terminology scans for removed public product names.
- Focused regression test for descriptor-ref `rpartition("@")` projection.
