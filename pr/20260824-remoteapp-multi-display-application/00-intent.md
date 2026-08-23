# Intent

Make RemoteApp application-surface support truthful at the multi-display seam.
Prove the existing process-scoped xcap compositor does not leak display pixels,
and expose the macOS single-display ScreenCaptureKit limit as an explicit
capability constraint instead of an implicit `production_ready` claim.
