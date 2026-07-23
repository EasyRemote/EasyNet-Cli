# SDK Directory answer-kind fail-closed convergence

## Goal

Remove the SDK-side provider-output fallback that inferred a Directory
`answer_kind` from the presence of a `negative` object.

## Root abstraction problem

Directory provider output is an authoritative runtime fact projection. The SDK
must not synthesize missing provider facts because downstream products then
cannot distinguish a complete negative answer from a malformed provider
projection.

## Invariants

1. Go and Python Directory projections require explicit `answer_kind` for every
   provider result.
2. A present `negative` object is preserved as a fact, not promoted into an
   answer state.
3. Missing `answer_kind` fails before product read models can interpret route or
   directory state.
4. Go and Python SDK behavior remains symmetric.
5. The SPEC gate covers the removed fallback so it cannot be reintroduced.

## Boundary proof

- Runtime providers own `answer_kind`.
- SDK Directory projections own validation and typed projection.
- Product read models consume SDK facts but do not get inferred authority state.

## Verification plan

- Go Directory tests.
- Python Directory tests.
- Canonical runtime convergence v2 gate.
- Rust fmt gate, because repository-level commit quality requires it.
- Codegraph sync/status after source edits.

## Results

- `go test . -run 'Test.*Directory'` from `sdk/go`: passed.
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python:sdk/python python -m pytest sdk/python/tests/test_directory.py -q`: passed.
- `cargo fmt --check`: passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`: passed.
- `tools/scripts/check-architecture-convergence.sh`: passed.
- `codegraph sync .` and `codegraph status .`: index synced and up to date.

## Delta

- Removed Go and Python SDK Directory fallback from `negative` to
  `RESOLVE_ANSWER_KIND_NEGATIVE`.
- Added Go/Python regression tests for negative output without explicit
  `answer_kind`.
- Extended SPEC v2 gate to reject the retired fallback and require the
  regression tests.
