# Python Addressing Transport Boundary Plan

## Objective

Allow Python consumer boundary audits to distinguish SDK-delegating
`AddressingTransport` adapter methods from raw consumer-owned URA or
DescriptorRef helper implementations.

This closes the gap introduced by exposing the package-level
`project_descriptor_ref` facade: downstream adapters may need a method with the
canonical transport name, but the method is acceptable only when it delegates to
the SDK facade instead of parsing or assembling addressing material itself.

## Invariants

- The SPEC remains unchanged.
- URA and DescriptorRef grammar ownership remains in Axon/SDK projection
  helpers.
- Consumers still cannot define raw addressing helper functions or assemble
  DescriptorRef strings.
- The exception applies only to `AddressingTransport` adapter methods that
  delegate to SDK identity helpers by the same method name.
- Public Python SDK source must not contain forbidden raw runtime boundary
  tokens such as direct raw ABI prefixes.

## Implementation Steps

1. Build a local AST parent map during addressing audit.
2. Recognize methods owned by `AddressingTransport` adapter classes.
3. Allow those methods only when they call the SDK identity facade by the same
   method name.
4. Add a focused cutover audit regression test.

## Boundary Proof

The audit still flags ordinary functions and non-SDK facade helpers named like
raw addressing helpers. The new allowance is structural rather than name-only:
the method must be inside an addressing transport class and must delegate to the
SDK facade. This keeps adapter shape legal without granting consumers ownership
over URA parsing or DescriptorRef projection.

## Verification Result

- PASS: `uv run python -m unittest tests.test_cutover_audit.ConsumerBoundaryAuditTests.test_allows_sdk_delegating_addressing_transport_methods`
- PASS: `uv run python -m unittest discover tests`
- PASS: `git diff --check`
- PASS: `git diff -- docs/spec/daemon-sdk-requirements-v1.md` produced no diff.
