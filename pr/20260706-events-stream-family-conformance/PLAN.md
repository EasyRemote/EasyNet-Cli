# Events Stream Family Conformance Plan

## Objective

Replace the stale `other_event_streams: scaffold_only` marker in
`events/session_stream` with provider-backed evidence from the existing Events
stream family.

The session stream case proves daemon `session.attach` carrier semantics; the
Directory stream and Device/Invocation history cases already prove the rest of
the canonical stream family across Go and Python shared conformance.

## Invariants

- The SPEC remains unchanged.
- Events remain a single stream-family facade: directory, device, session, and
  invocation streams share carrier validation, cursor semantics, and lifecycle
  state.
- The SDK must not introduce a local event bus or product-specific session URA
  parser.
- Runtime ownership stays below the Events facade through Runtime Core / C ABI
  transports.

## Implementation Steps

1. Promote `other_event_streams` from scaffold-only to provider-backed.
2. Assert that expectation in Go and Python shared session-stream conformance.
3. Re-run focused Events shared conformance plus full Go/Python/runner gates.

## Boundary Proof

Go `EventsRuntimeTransport` maps directory, device, session, and invocation
streams to runtime subscription carriers and opens runtime streams. Python
`CABIEventTransport` builds the same subscription carriers and opens Runtime
Core streams for directory, device, session, and invocation. The shared
directory and device/invocation cases provide concrete evidence for the
non-session stream family rather than leaving `events/session_stream` as an
isolated scaffold.

## Verification Plan

- `go test ./... -run TestGoEventsFacadeExecutesShared`
- `uv run python -m unittest tests.test_conformance.SharedConformanceFixtureTests.test_python_events_executes_shared_directory_stream_conformance_case tests.test_conformance.SharedConformanceFixtureTests.test_python_events_executes_shared_session_stream_conformance_case tests.test_conformance.SharedConformanceFixtureTests.test_python_events_executes_shared_device_invocation_history_case`
- `go test ./...`
- `uv run python -m unittest discover tests`
- `cargo test --bin sdk-conformance-runner`
- Go/Python `sdk-conformance-runner` adapter reports
- `bash tools/scripts/check-sdk-scaffold.sh`
- `git diff --check`
- `git diff -- docs/spec/daemon-sdk-requirements-v1.md`

## Verification Result

- PASS: `go test ./... -run TestGoEventsFacadeExecutesShared`
- PASS: `uv run python -m unittest tests.test_conformance.SharedConformanceFixtureTests.test_python_events_executes_shared_directory_stream_conformance_case tests.test_conformance.SharedConformanceFixtureTests.test_python_events_executes_shared_session_stream_conformance_case tests.test_conformance.SharedConformanceFixtureTests.test_python_events_executes_shared_device_invocation_history_case`
- PASS: `go test ./...`
- PASS: `uv run python -m unittest discover tests`
- PASS: `cargo test --bin sdk-conformance-runner`
- PASS: `cargo run --bin sdk-conformance-runner -- --language go --adapter-report sdk/conformance/runner/go-action-adapter-report.json --format jsonl`
- PASS: `cargo run --bin sdk-conformance-runner -- --language python --adapter-report sdk/conformance/runner/python-action-adapter-report.json --format jsonl`
- PASS: `bash tools/scripts/check-sdk-scaffold.sh`
- PASS: `git diff --check`
- PASS: `git diff -- docs/spec/daemon-sdk-requirements-v1.md` produced no diff.
