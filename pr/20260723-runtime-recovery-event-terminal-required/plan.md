# Runtime recovery event terminal fact convergence

## Goal

Make Go SDK restart-recovery event projection fail closed when the provider
omits the `terminal` lifecycle fact, matching the Python SDK and the canonical
runtime state-machine model.

## Root abstraction problem

`terminal` is not an optional display field. It is part of the recovery event
state machine and determines whether a recovery observation is closed. The Go
SDK decoded a missing boolean as `false`, which is a provider-output fallback.

## Invariants

1. Go and Python recovery events require `sequence`, `kind`, and `terminal`.
2. Missing `terminal` must fail before a product consumes the event.
3. Recovery report counters and ready-state proof remain unchanged.
4. SPEC v2 rejects reintroduction of implicit `terminal=false`.

## Boundary proof

- Runtime providers own recovery lifecycle facts.
- SDK projection validates lifecycle facts and exposes typed events.
- Products consume a fully explicit state machine; they do not infer missing
  terminality.

## Verification plan

- Go runtime focused tests.
- Python runtime focused tests for parity.
- Repository rustfmt check.
- Canonical runtime convergence v2 gate.
- Legacy architecture gate.
- Codegraph sync/status.

## Results

- `go test . -run 'TestRuntime.*|TestInvocation.*|TestPrepared.*'` from `sdk/go`: passed.
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python:sdk/python python -m pytest sdk/python/tests/test_runtime.py -q`: passed.
- `cargo fmt --check`: passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`: passed.
- `tools/scripts/check-architecture-convergence.sh`: passed.
- `codegraph sync .` and `codegraph status .`: index up to date.

## Delta

- Added a private Go runtime recovery event wire DTO with `terminal` as a
  required pointer-backed fact.
- Kept the public `RuntimeRecoveryEvent` API stable.
- Added Go/Python regression coverage for missing recovery event terminality.
- Added a SPEC v2 SDK runtime recovery contract.
- Regenerated SDK conformance attestation after the Go runtime provider source
  changed.
