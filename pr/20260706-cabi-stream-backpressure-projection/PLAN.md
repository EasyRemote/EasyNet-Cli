# C ABI Stream/Bidi Backpressure Projection Plan

## Objective

Close the Runtime Core C ABI stream/bidi backpressure projection gap without
changing the daemon SDK requirements SPEC or adding product-specific stream
adapters.

## Invariants

- The SPEC remains unchanged.
- Axon remains the owner of stream/bidi protocol state machines and canonical
  frame semantics.
- EasyNet-Cli owns the language-binding callback queue and may expose local
  callback-queue overflow as a typed Runtime Core terminal projection.
- The projection must be shared under `src/protocol/` so C ABI, Go, Python, and
  future bindings converge on one JSON shape.
- Overflow must fail closed: bounded memory, one terminal error projection, and
  local reader teardown.
- Existing stream/bidi public symbols and callback signatures remain unchanged.

## Implementation Steps

1. Add a shared Runtime stream/bidi contract module for typed terminal
   backpressure projections.
2. Refactor C ABI stream and bidi down readers to use nonblocking bounded
   callback queue sends and emit terminal backpressure projections on overflow.
3. Add Rust unit tests for stream/bidi projection shape and reader overflow
   behavior where feasible without daemon I/O.
4. Add/update conformance evidence and parity notes for C ABI bounded
   backpressure.
5. Run targeted Rust invocation tests, Go/Python stream/bidi tests, hygiene, and
   SPEC diff.

## Boundary Proof

This slice does not implement Axon stream admission, remote flow control, or a
product wrapper stream bridge. It only turns a local C ABI binding queue
overflow into a stable Runtime Core terminal DTO. Backpressure policy remains
local and bounded; daemon protocol semantics remain delegated to Axon/daemon.

## Verification Plan

- `cargo test invocation_ --lib`
- `go test ./... -run 'Stream|Bidi|CABI'`
- `uv run python -m unittest tests.test_stream tests.test_bidi tests.test_cabi`
- `cargo fmt --check`
- `git diff --check`
- `git diff -- docs/spec/daemon-sdk-requirements-v1.md`

## Verification Result

- PASS: `python -m json.tool` for SDK parity matrix and action-adapter reports
- PASS: `cargo test runtime_stream_contract --lib`
- PASS: `cargo test bounded_callback_enqueue --lib`
- PASS: `go test ./... -run 'Stream|Bidi|Conformance'`
- PASS: `uv run python -m unittest tests.test_stream tests.test_bidi tests.test_conformance`
- PASS: `cargo fmt --check`
- PASS: `git diff --check`
- PASS: `git diff -- docs/spec/daemon-sdk-requirements-v1.md` produced no diff.
