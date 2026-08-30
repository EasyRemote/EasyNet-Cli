# Verification

Passed on macOS with the physical built-in camera:

- `cargo check --lib`;
- `cargo check --all-targets` (passed; pre-existing/concurrent RemoteApp
  dead-code warnings only);
- camera focused tests: 18 passed;
- native `AVCapturePhotoOutput` smoke after supported AF/AE/AWB configuration
  and bounded convergence: 63,528-byte 1920x1080 JPEG, visually confirmed as a
  real camera image;
- native preview smoke: three live JPEG frames, bounded latest-value lane;
- native recording automatic duration stop: H.264/MOV finalized successfully;
- native recording explicit user stop: H.264/MOV finalized successfully after
  readiness, with non-zero frame count and `stop_reason=stopped`;
- `git diff --check`.
- raw live-camera browser framing tests preserve JPEG bytes across fragmented
  binary records without a base64 field;
- backend focused tests pass after preferring SDK raw payload bytes before the
  legacy base64 projection;
- `camera.snapshot` compatibility response decodes to JPEG SOI/EOI bytes
  identical to the committed Context media object.

Existing RemoteApp warnings and concurrently modified files are outside this
task.

Pending for a fully raw browser unary snapshot:

- a new additive unary binary C ABI extension (v9 is stream-lease-only and must
  not be silently widened);
- Go/Python owned-byte bindings and HTTP unary binary content negotiation;
- physical browser invocation proving the raw unary JPEG does not traverse the
  legacy `output_base64` result projection.
