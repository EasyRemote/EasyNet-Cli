# Surface Health DescriptorRef Delegation Plan

## Objective

Remove the remaining Go Surface runtime facade-side DescriptorRef construction
for `pages.health` and project the health DescriptorRef from the
identity-built Invocation instead.

## Boundary

- Do not change `docs/spec/daemon-sdk-requirements-v1.md`.
- Do not reimplement Axon DescriptorRef grammar in the Go SDK.
- Do not change backend rendering ownership or surface content policy.
- Keep the Surface runtime transport as a facade over Runtime Core plus
  Directory/Identity.

## Invariants

1. Surface Runtime Core invocation construction obtains DescriptorRefs through
   `IdentityClient.OwnerAbilityDescriptorRef`.
2. Surface health projection never builds `ability_ura@version` locally.
3. If daemon output includes a DescriptorRef, preserve it; otherwise use the
   DescriptorRef already attached to the identity-built Invocation.
4. Missing DescriptorRef facts fail closed with typed Surface profile errors.
5. Shared Surface conformance must record the DescriptorRef source.

## Implementation Steps

1. Inspect the current Surface runtime `pages.health` projection path.
2. Thread the identity-built Invocation descriptor ref into health projection.
3. Remove the `fmt.Sprintf("%s@%s")` fallback.
4. Add Go tests that prove `pages.health` uses the identity transport.
5. Update the shared Surface conformance case with the descriptor source.

## Verification

- `go test ./... -run 'Surface|Conformance|Identity'`
- `PYTHONPATH=tests uv run python -m unittest tests.test_surface tests.test_conformance`
- `tools/scripts/check-sdk-parity-matrix.sh --self-test`
- `tools/scripts/check-sdk-scaffold.sh`
- `git diff --check`
