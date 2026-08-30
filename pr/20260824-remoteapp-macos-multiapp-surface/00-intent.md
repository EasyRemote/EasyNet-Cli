# Intent

Implement a real macOS RemoteApp application surface that captures the complete
committed application window set across displays. Replace the current
display-scoped application inventory and single ScreenCaptureKit application
filter with a bounded multi-window native capture/composition source feeding
the existing VideoToolbox/WebRTC pipeline.
