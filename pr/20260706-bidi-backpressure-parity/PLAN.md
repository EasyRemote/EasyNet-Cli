# Bidi Backpressure Parity Evidence Plan

## Objective

Align the SDK parity evidence with the current Runtime Core implementation:
`stream-backpressure-bound.yaml` already exercises both stream and bidi callback
queue overflow terminal projection, so the bidi capability row must cite the
same shared case as the stream row.

## Boundary

- Do not change `docs/spec/daemon-sdk-requirements-v1.md`.
- Do not introduce SDK-side stream or bidi semantics.
- Do not alter C ABI, Go, or Python behavior unless the audit finds a real
  implementation mismatch.
- Keep Axon-owned stream/bidi terminal semantics delegated through the existing
  Rust/C ABI projection and language facade conformance tests.

## Invariants

1. Stream and bidi queue overflow remain bounded and deterministic.
2. Each stream/session terminal event remains single-closure and typed.
3. Go and Python parity claims reference the same conformance evidence when the
   shared case covers both capabilities.
4. Remaining-work text must distinguish daemon SDK coverage from external
   product cutover work.

## Implementation Steps

1. Audit the shared conformance case and parity matrix rows.
2. Add `stream-backpressure-bound.yaml` to the bidi shared cases when confirmed.
3. Update SDK parity Markdown summary if its bidi row understates current daemon
   SDK coverage.
4. Run the parity matrix self-test and targeted Go/Python conformance checks.

## Verification

- `tools/scripts/check-sdk-parity-matrix.sh --self-test`
- `go test ./... -run 'ParityMatrix|Conformance|StreamBackpressure'`
- `PYTHONPATH=tests uv run python -m unittest tests.test_conformance`
- `cargo test runtime_stream_contract --lib`
- `git diff --check`

## Result

- Added the shared `stream-backpressure-bound.yaml` conformance case to the
  bidi parity row because the case includes both bidi overflow and bidi
  terminal projection actions.
- Updated the SDK parity summary so bidi no longer understates existing C ABI
  terminal and bounded backpressure coverage.
- Left external product wrapper stream adapters as remaining work outside the
  daemon SDK.
