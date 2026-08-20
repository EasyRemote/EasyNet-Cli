# Directory Subscription Conformance Plan

## Objective

Promote Directory subscribe convenience from local Go/Python facade coverage to
shared conformance evidence.

The SDK already exposes `DirectorySubscriptionRequest`,
`BuildDirectorySubscriptionInvocation`, `SubscribeDirectory`, bounded buffered
events, resume cursors, and subscription state-machine helpers. The remaining
gap is that the shared conformance suite does not yet prove those surfaces in
both P0 language facades.

## Invariants

- The SPEC remains unchanged.
- Axon/daemon own the `directory.subscribe` Invocation semantics; SDK facades
  only build complete carriers and project daemon subscription state.
- Subscription projection must preserve cursor, resume token, drop count,
  terminal state, and bounded event buffering.
- Live events require a prior snapshot completion.
- The SDK must not implement facade-side fan-out or invent directory stream
  semantics.
- Go and Python must consume the same case, fixtures, and schemas.

## Implementation Steps

1. Add shared Directory subscription request, Invocation, and projection
   fixtures plus schemas.
2. Register fixture/schema/case files in the scaffold and schema-binding gates.
3. Extend Go shared conformance to execute subscription build, subscribe, and
   projection checks.
4. Extend Python shared conformance to execute the same fixture-backed actions.
5. Record Go/Python adapter-report evidence and update parity documentation.

## Boundary Proof

`directory.subscribe` is daemon-owned because it opens a daemon directory stream
over a complete Invocation carrier. The language SDKs do not select peers,
fan-out subscriptions, or synthesize directory events. They only expose the
carrier builder and a typed, bounded projection over daemon-emitted stream
state.

## Verification Plan

- `go test ./... -run TestGoDirectoryIdentityFacadeExecutesSharedProjectionConformanceCases`
- `uv run python -m unittest tests.test_conformance.SharedConformanceFixtureTests.test_python_directory_identity_execute_shared_projection_cases`
- `go test ./...`
- `uv run python -m unittest discover tests`
- `cargo test --bin sdk-conformance-runner`
- `cargo run --bin sdk-conformance-runner -- --language go --adapter-report sdk/conformance/runner/go-action-adapter-report.json --format jsonl`
- `cargo run --bin sdk-conformance-runner -- --language python --adapter-report sdk/conformance/runner/python-action-adapter-report.json --format jsonl`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `git diff --check`
- `git diff -- docs/spec/daemon-sdk-requirements-v1.md`

## Verification Result

- PASS: `go test ./... -run TestGoDirectoryIdentityFacadeExecutesSharedProjectionConformanceCases`
- PASS: `uv run python -m unittest tests.test_conformance.SharedConformanceFixtureTests.test_python_directory_identity_execute_shared_projection_cases`
- PASS: `bash tools/scripts/check-sdk-scaffold.sh`
- PASS: `go test ./...`
- PASS: `uv run python -m unittest discover tests`
- PASS: `cargo test --bin sdk-conformance-runner`
- PASS: `cargo run --bin sdk-conformance-runner -- --language go --adapter-report sdk/conformance/runner/go-action-adapter-report.json --format jsonl`
- PASS: `cargo run --bin sdk-conformance-runner -- --language python --adapter-report sdk/conformance/runner/python-action-adapter-report.json --format jsonl`
- PASS: `git diff --cached --check && git diff --check`
- PASS: `git diff -- docs/spec/daemon-sdk-requirements-v1.md && git diff --cached -- docs/spec/daemon-sdk-requirements-v1.md` produced no diff.

## Verification Result

- PASS: `go test ./... -run TestGoDirectoryIdentityFacadeExecutesSharedProjectionConformanceCases`
- PASS: `uv run python -m unittest tests.test_conformance.SharedConformanceFixtureTests.test_python_directory_identity_execute_shared_projection_cases`
- PASS: `go test ./...`
- PASS: `uv run python -m unittest discover tests`
- PASS: `cargo test --bin sdk-conformance-runner`
- PASS: `cargo run --bin sdk-conformance-runner -- --language go --adapter-report sdk/conformance/runner/go-action-adapter-report.json --format jsonl`
- PASS: `cargo run --bin sdk-conformance-runner -- --language python --adapter-report sdk/conformance/runner/python-action-adapter-report.json --format jsonl`
- PASS: `bash tools/scripts/check-sdk-scaffold.sh`
- PASS: `git diff --check`
- PASS: `git diff -- docs/spec/daemon-sdk-requirements-v1.md` produced no diff.
