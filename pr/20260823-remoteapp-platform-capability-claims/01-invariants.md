# Invariants — RemoteApp Platform Capability Claims

1. The plugin manifest may describe package scope, but product UI must consume
   runtime capability projection.
2. `production_target_subjects` means subjects served by a currently
   production-ready backend, not subjects listed by an unavailable descriptor.
3. Diagnostic transport subjects must stay separate from production subjects.
4. On non-macOS, macOS ScreenCaptureKit/VideoToolbox descriptors may remain in
   the catalogue as `not_installed`, but they must not advertise product-ready
   window/application capture.
5. On macOS without Screen Recording permission, production subjects must be
   empty until the native backend is actually production-ready.
6. Display diagnostic fallback must not imply app/window fallback.
7. This does not complete Windows/Linux capture; it only makes the unsupported
   product state explicit and machine-readable.
