# Intent

Close the SDK conformance evidence drift that prevents live parity validation
from proving the current source tree.

The runner report for the Rust bidi close-send case still points at an older
`src/ffi/invocation/mod.rs` hash. The live result was captured from the current
source snapshot, so `check-sdk-parity-matrix.sh` correctly rejects the proof as
`evidence_hash_mismatch`.

## Non-goals

- Do not relax evidence hash validation.
- Do not make parity accept stale runner snapshots.
- Do not change runtime, SDK, or ABI behavior.

## Acceptance Criteria

- Runner-owned evidence hashes match the current source files they reference.
- `canonical-public-api.json` and `sdk-parity-matrix.json` are regenerated from
  the canonical model generator.
- Live parity validation accepts the existing live result directory.
