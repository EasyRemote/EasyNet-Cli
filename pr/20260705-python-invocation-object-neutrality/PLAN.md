# Python Invocation Object Neutrality

## Objective

Remove EasyRemote-specific public naming from the Python Runtime Core invocation object adapter while preserving canonical seven-field Invocation DTO construction required by `docs/spec/daemon-sdk-requirements-v1.md`.

## Boundary Proof

- Ownership: tuple/object adaptation into `InvocationDraft` belongs to Runtime Core as a generic language facade helper, not to an EasyRemote product facade.
- Protocol: the adapter still builds complete caller/callee/ability/subject/nonce/causal_context/args requests through `AbilityInvocationClient`.
- Delegation: descriptor reference and Ability URA projection continue to use `AddressingClient`; no parser or canonicalization logic is duplicated.
- Compatibility posture: the product-named public adapter is removed rather than aliased so the SDK exposes one canonical invocation object adapter.

## Implementation

- Rename `EasyRemoteInvocationAdapter` to `InvocationObjectAdapter`.
- Update adapter docstrings and validation messages to generic invocation object terminology.
- Update exports, tests, conformance fixture expectations, and SDK docs.

## Verification

- Python cutover and conformance tests.
- Python SDK test suite.
- Go SDK tests.
- SDK scaffold gate.
- Formatting, diff, and terminology scans.
