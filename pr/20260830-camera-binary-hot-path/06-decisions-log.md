# Decisions log

## 2026-08-30

- Use existing Axon/Runtime raw-payload semantics rather than inventing a
  camera-specific transport.
- Add a typed bounded live carrier instead of misusing `TypedFinite`; media is
  live until cancellation and its name must preserve lifecycle meaning.
- Keep one latest-value watch slot before the carrier. Reliable delivery of old
  viewfinder frames increases latency and has no product value.
- Commit finalized recordings by file ownership transfer; never materialize a
  complete movie as a `Vec<u8>`.
- Treat backend SSE base64 as a separate remaining product seam. Removing the
  daemon base64 projection is necessary but not sufficient for browser-level
  zero-base64 delivery.
- Use `AVCapturePhotoOutput` for still capture. A preview frame is not a photo;
  the native photo delegate, quality-priority, and terminal callbacks define
  the correct capture lifecycle.
- A native photo API alone is insufficient: configure only supported center
  point continuous autofocus, auto exposure, and auto white balance while the
  device configuration lock is held, then wait for a bounded warm-up/stability
  window. Use `Balanced` so low-light photo processing is not disabled merely
  to minimize callback time.
- Reject `AVCaptureMovieFileOutput` for daemon recording after physical tests
  showed its mandatory completion delegate depended on application-main-thread
  behavior. Use `AVCaptureVideoDataOutput` plus `AVAssetWriter` on a dedicated
  serial dispatch queue instead.
- Request native NV12 (`420v`) recording buffers because Apple's H.264 encoder
  consumes that format directly. Keep BGRA only where JPEG preview conversion
  is required.
- Feed the JPEG encoder through a strided pixel-buffer view instead of copying
  every frame into a contiguous RGB allocation.
- Treat `descriptor.v9` as a descriptor fixture version, not as a transport ABI
  promise. The shipped C ABI is v7 plus an additive v8 stream-frame extension;
  protobuf unary results are already raw bytes.
- Fix typed unary output in Axon's canonical Runtime result rather than
  converting `camera.snapshot` into a one-frame stream or adding a camera-only
  transport bypass.
