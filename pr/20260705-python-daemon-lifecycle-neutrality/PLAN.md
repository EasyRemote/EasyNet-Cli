# Python Daemon Lifecycle Neutrality

## Objective

Remove product-specific EasyRemote daemon lifecycle naming from the Python daemon SDK while preserving the Runtime Core daemon lifecycle behavior required by `docs/spec/daemon-sdk-requirements-v1.md`.

## Boundary Proof

- Ownership: Python remains a language facade over daemon SDK Runtime Core. It may expose typed projection objects, but it must not encode EasyRemote product identity in public daemon lifecycle names.
- State: daemon lifecycle still flows through `StartConfig`, `DaemonControl`, and `DaemonHandle`; no new state machine or terminal-state taxonomy is introduced.
- Transport: start/status/open-runtime behavior continues to delegate to `DaemonTransport`; no Axon protocol or daemon policy semantics are reimplemented in Python.
- Compatibility posture: old product-named Python symbols are removed rather than aliased, so the SDK converges on one public daemon model.

## Implementation

- Rename the product-specific start projection to `DaemonStartProjection`.
- Rename lifecycle and handle projections to `DaemonLifecycleFacade` and `DaemonHandleFacade`.
- Normalize public wire projection fields to daemon SDK terms: `device_id` and `detached`.
- Normalize the Python C ABI daemon-start adapter to forward `device_id` and `detached` instead of legacy product fields.
- Move validation errors onto the existing `daemon_lifecycle` stage.
- Update exports, tests, and SDK parity docs.

## Verification

- Python daemon tests.
- Python SDK test suite.
- Go test suite.
- SDK scaffold gate.
- Formatting/terminology scans for product-specific daemon lifecycle leftovers.
