# Decisions Log

- 2026-07-16: Treat the existing conformance JSON delta as stale generated
  output after regeneration removed the diff; do not commit transient JSON.
- 2026-07-16: Move the invariant into `check-sdk-canonical-public-api.sh` so
  stale generated public API or parity matrix output is rejected by the SDK
  cutover gate instead of depending on manual regeneration.
- 2026-07-16: Keep this slice limited to SDK conformance gates and proof
  documentation; no public API or provider state promotion is allowed here.
