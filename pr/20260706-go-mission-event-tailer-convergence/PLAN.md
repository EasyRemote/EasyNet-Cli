# Go Mission Event Tailer Convergence Plan

## Objective

Close the Go/Python Mission facade parity gap for bounded Mission event live
tailing without changing `daemon-sdk-requirements-v1.md` or moving daemon-owned
Mission policy into the SDK.

## Invariants

- The SPEC remains unchanged.
- Mission events are still fetched through `MissionClient.Events`; the SDK does
  not invent a separate daemon stream protocol.
- The tailer is a facade-level state machine over paged daemon Mission events:
  cursor progress is explicit, terminal events close the tail, and dropped pages
  fail as protocol errors.
- Terminal closure is single-shot: events returned after a terminal event in the
  same daemon page are not surfaced by the SDK tailer.
- Mission IDs remain opaque identifiers, not paths or product session URAs.
- Tail bounds are explicit: limit, empty-page budget, and polling delay are
  caller-controlled and validated.
- Go and Python expose equivalent Mission event tail semantics while preserving
  the existing public Mission operations.

## Implementation Steps

1. Add Go `MissionEventTailOptions` and `MissionEventTailer` to the Mission
   facade with explicit cursor, buffer, terminal, and closed states.
2. Add `MissionClient.TailEvents` as the canonical Go entry point for this
   facade-level state machine.
3. Add focused Go/Python tests for terminal tailing, cursor propagation,
   terminal-in-page hard stop, dropped-event protocol errors, no-progress guard,
   and validation.
4. Update shared conformance/coverage assertions to acknowledge live-tail
   support instead of leaving the Go-facing profile as scaffold-only.
5. Map Python `MissionClient.tail_events` to the Mission profile ownership gate
   so both languages expose the live-tail facade without MEMC ambiguity.
6. Run Go/Python unit and conformance gates, SPEC diff, and whitespace checks
   before committing.

## Boundary Proof

The daemon owns Mission execution, event persistence, and event-page projection.
The SDK tailer owns only client-side iteration over those pages. It does not
construct child Invocations, infer Mission state, fabricate receipts, or parse
product-specific session addresses. The state machine is intentionally bounded:
it either yields ordered events, stops on terminal/empty budget/close, or fails
on dropped events and cursor non-progress. Once a terminal event is observed,
the SDK marks the tail closed and does not expose any later event from that
page, preserving a single terminal closure even if a daemon page is malformed.

## Verification Plan

- `go test ./...` in `sdk/go`
- `uv run python -m unittest discover tests` in `sdk/python`
- `cargo test --bin sdk-conformance-runner`
- `cargo run --bin sdk-conformance-runner -- --language go --adapter-report
  sdk/conformance/runner/go-action-adapter-report.json --format jsonl`
- `cargo run --bin sdk-conformance-runner -- --language python --adapter-report
  sdk/conformance/runner/python-action-adapter-report.json --format jsonl`
- `git diff --check`
- `git diff -- docs/spec/daemon-sdk-requirements-v1.md`

## Verification Results

- `go test ./...` in `sdk/go`: passed.
- `uv run python -m unittest discover tests` in `sdk/python`: 455 tests
  passed.
- `cargo test --bin sdk-conformance-runner`: 9 tests passed.
- Go adapter report through `sdk-conformance-runner`: passed, including
  `mission/carrier_status`.
- Python adapter report through `sdk-conformance-runner`: passed, including
  `mission/carrier_status`.
- `git diff --check`: passed.
- `git diff -- docs/spec/daemon-sdk-requirements-v1.md`: empty.
