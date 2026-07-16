# Invariants

## Evidence Binding

- Every action-adapter report entry must hash the exact current bytes of its
  `ref_path` when the report gate runs.
- `canonical-public-api.json` shape hashes and `sdk-parity-matrix.json`
  step-shape evidence must match generator output from the same current source
  tree.
- The parity matrix state machine remains unchanged: this slice may refresh
  hashes, but it must not change capability state, provider proof references, or
  public SDK behavior.

## Runtime Boundary

- Stream and bidi cancellation remain SDK lifecycle state-machine behavior.
- The evidence ledger may attest the current implementation, but must not create
  a second provider, fallback parser, or compatibility alias.

## Verification Boundary

- `check-sdk-conformance-reports.sh` is the authoritative hash/evidence gate for
  action-adapter reports.
- `check-sdk-canonical-public-api.sh` must reject stale generated
  `canonical-public-api.json` and `sdk-parity-matrix.json` output before SDK
  cutover readiness can pass.
- `check-sdk-parity-matrix.sh` remains the live-result matrix validation gate.
