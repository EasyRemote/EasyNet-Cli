# Decisions log

## 2026-08-24

- Begin with provider inventory because the product goal cannot be satisfied by
  changing readiness labels or adding synthetic evidence.
- The root implementation gap is not resource discovery: xcap already discovers
  Windows/Linux windows. Session admission rejects those bindings, the media
  selector advertises xcap as display-only, application capture selects one
  primary window, and the platform observer reports unsupported. Close these as
  one provider seam so discovery, proof, streaming, and tracking agree.
- Keep the xcap path `production_ready=false` until real Windows and Linux host
  artifacts pass the cross-platform E2E contract. Executable baseline and
  product certification are separate facts.
- Model macOS application capture as display-scoped because ScreenCaptureKit
  filters require that identity, but model Windows/Linux xcap application
  capture as process-scoped. Do not invent a display id merely to reuse the
  macOS projection.
- Reject a disappeared or owner-drifted exact window rather than selecting a
  same-name replacement. Application membership changes enter the existing
  target rebind state machine.
