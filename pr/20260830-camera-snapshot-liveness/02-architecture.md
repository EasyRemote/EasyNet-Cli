# Architecture

`NokhwaBackend` owns `ActiveCameraStreams`. On macOS, `capture_jpeg` first reads
the latest frame only when that producer still has consumers. Otherwise it
acquires a temporary consumer from the same stream factory and waits for the
first frame. The separate `AVCapturePhotoOutput` delegate/session is removed.
