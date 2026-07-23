# SDK Directory record alias removal

## Goal

Remove SDK-side Directory record field aliases that projected old provider
shapes into canonical record fields.

## Root abstraction problem

Directory records are provider facts. The SDK should preserve raw provider
facts and project only canonical field names into typed convenience fields.
Accepting aliases such as `type` for `kind` or `canonical_name` for `ura`
lets old record shapes look canonical to products.

## Invariants

1. Go and Python Directory record projections only read `kind` into
   `DirectoryRecord.kind`.
2. Go and Python Directory record projections only read `ura` into
   `DirectoryRecord.ura`.
3. Retired aliases remain available only in `raw`, not in canonical typed
   fields.
4. Shared helper APIs no longer accept multiple field names for projection.
5. SPEC v2 rejects reintroduction of record alias projection.

## Boundary proof

- Runtime providers own record shape.
- SDK Directory projection owns canonical field validation/projection.
- Product read models can inspect `raw` for diagnostics but cannot receive
  alias-promoted facts as canonical fields.

## Verification plan

- Go Directory focused tests.
- Python Directory focused tests.
- Repository rustfmt check.
- Canonical runtime convergence v2 gate.
- Legacy architecture gate.
- Codegraph sync/status.

## Results

- `go test . -run 'Test.*Directory'` from `sdk/go`: passed.
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python:sdk/python python -m pytest sdk/python/tests/test_directory.py -q`: passed.
- `cargo fmt --check`: passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`: passed.
- `tools/scripts/check-architecture-convergence.sh`: passed.
- `codegraph sync .` and `codegraph status .`: index synced and up to date.

## Delta

- Replaced the Go SDK Directory text projector with a single-field helper.
- Replaced the Python SDK Directory text projector with a single-field helper.
- Removed `type -> kind` and `canonical_name -> ura` Directory record alias promotion.
- Added Go/Python regression tests proving aliases remain only in `raw`.
- Extended SPEC v2 to reject the retired alias projectors and require tests.
