# Intent

## Goal

Make SDK conformance generated-output freshness executable after stale
stream/bidi report and matrix hashes appeared in the working tree but vanished
when regenerated from the current source.

## Expected Effect

- Effect type: architecture convergence.
- Root fork addressed: SDK provider evidence drift between implementation
  source, action-adapter reports, canonical public API shape hashes, and the SDK
  parity matrix.
- Concrete use case: SDK cutover readiness must reject stale proof hashes, but
  should not rely on manual inspection to notice stale generated
  `canonical-public-api.json` or `sdk-parity-matrix.json` output.

## Non-goals

- Do not change SDK runtime behavior.
- Do not promote any `seam` cell to `provider-backed` or `cutover-ready`.
- Do not commit transient regenerated JSON when the current generator output
  already matches the tracked files.
- Do not edit unrelated dirty docs, specs, packaging, or skill files.
