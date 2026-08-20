# Runtime recovery counters required-fact convergence

## Goal

Make restart-recovery counters explicit provider facts in both Go and Python
SDKs. Missing counters must fail closed instead of being projected as zero.

## Root abstraction problem

Recovery counters are audit facts proving the restart recovery scan and replay
outcome. A missing `recovered_invocations`, `reaped_orphans`, or
`replayed_terminal_receipts` field is not equivalent to zero; treating it as
zero hides incomplete provider reports from products.

## Invariants

1. Go and Python recovery reports require all three recovery counters.
2. Counters remain non-negative integers.
3. Public report field names and types remain stable.
4. SPEC v2 rejects implicit zero counter projection.

## Boundary proof

- Runtime providers own restart-recovery audit facts.
- SDK projections validate those facts and expose a typed report.
- Products consume explicit counters and do not infer missing recovery work.

## Verification plan

- Go runtime focused tests.
- Python runtime focused tests.
- Repository rustfmt check.
- Canonical runtime convergence v2 gate.
- Canonical public API gate.
- Legacy architecture gate.
- Codegraph sync/status.

## Decisions

- Treat recovery report counters as required provider-owned audit facts, not
  optional presentation values.
- Preserve public field names and public field types while tightening SDK
  decoding semantics.
- Make the SPEC v2 gate reject future implicit-zero projections in both Go and
  Python.

## Delta

- Go recovery report wire DTO now decodes the three counters as nullable
  internal fields and projects them only through a required non-negative
  counter validator.
- Python recovery report decoding now uses a required non-negative counter
  validator instead of optional-zero conversion for the same facts.
- Go and Python focused tests now cover missing and negative recovery counter
  reports.
- Canonical API and parity attestations were regenerated after the SDK
  implementation change.

## Results

- `go test . -run 'TestRuntime.*|TestInvocation.*|TestPrepared.*'`
  passed in `sdk/go`.
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python:sdk/python python -m pytest sdk/python/tests/test_runtime.py -q`
  passed: 36 tests.
- `cargo fmt --check` passed.
- `tools/scripts/check-architecture-convergence.sh` passed.
- `tools/scripts/check-sdk-canonical-public-api.sh` passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh` passed.
- `codegraph sync . && codegraph status .` completed with an up-to-date index.
