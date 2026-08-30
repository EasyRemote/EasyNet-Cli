# Invariants — RemoteApp Platform Support Matrix

1. Platform support is product capability metadata, not an AbilityDescriptor
   owner or invocation tuple field.
2. macOS display/window/application capture is tied to the native
   ScreenCaptureKit/VideoToolbox production gate.
3. Linux display is diagnostic-only until a production backend exists.
4. Linux window/application capture is unsupported until a native target
   observer and media backend exist.
5. Windows display/window/application capture is unsupported until a native
   backend exists.
6. Unsupported rows must expose stable reasons suitable for frontend UI and E2E
   assertions.
