# Authority Minting Facade Intent

## Goal

Expose typed authority minting through the EasyNet-Cli SDK so EasyNet backend
and EasyRemote can request `DelegationProof` and `SessionAuthority` metadata
without importing Axon SDK packages or owning raw authority wire semantics.

## Non-Goals

- Do not change `docs/spec/daemon-sdk-requirements-v1.md`.
- Do not reimplement Axon canonical authority payload algorithms in Go or
  Python.
- Do not expose raw Axon SDK, protobuf, or daemon transport types in the public
  language SDK surface.
- Do not claim backend cutover-ready until the EasyNet backend import-ban gate
  passes against the sibling repository.

## Acceptance Criteria

- Go SDK exposes an `AuthorityClient` with typed delegation/session minting
  requests and typed projections.
- Python SDK exposes the same authority minting facade shape.
- Authority metadata remains mutually exclusive when attached to Invocation
  builders.
- Tests prove that minting goes through a provider/transport boundary and that
  invalid requests fail before any transport call.
- SDK parity documentation records this as a facade capability, with remaining
  concrete daemon/product cutover work explicit.
