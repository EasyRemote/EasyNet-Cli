# Execution checklist

- [x] Inventory capture, daemon Runtime, SDK v8, backend SSE, and browser paths.
- [x] Prove current queue and copy complexity.
- [x] Add bounded typed-live Runtime source.
- [x] Project camera preview directly to typed JPEG frames.
- [x] Add constant-memory Context file commit.
- [x] Switch recording stop to file commit.
- [x] Update camera stream descriptor contract.
- [x] Replace macOS recording with native NV12 -> AVAssetWriter H.264/MOV.
- [x] Configure supported native AF/AE/AWB controls and wait for bounded
  convergence before still capture.
- [x] Use Balanced native photo processing rather than speed-at-any-cost.
- [x] Remove the preview's full-frame RGB staging allocation.
- [x] Add backpressure, byte-identity, rollback, and persistence tests.
- [x] Run focused verification.
- [x] Run physical native photo, preview, and recording smoke.
- [x] Re-run `cargo check --all-targets` after final review.
- [x] Add canonical Axon/daemon support for typed unary output without JSON
  projection, while preserving existing byte-only provider APIs.
- [x] Keep `camera.snapshot` on its established JSON/base64 public result until
  an additive raw-unary ABI and browser binding exist; do not ship a daemon-only
  raw result that the product cannot consume.
- [x] Update the snapshot descriptor to describe native capture and its current
  compatibility result honestly.
- [x] Remove base64 from the live camera browser path: raw SDK bytes -> bounded
  binary HTTP records -> `Uint8Array` -> Blob URL.
- [x] Revoke replaced and closed camera Blob URLs so retained preview memory is
  O(1).
- [x] Add explicit Go SDK stream acknowledgement and have Backend acknowledge
  each delivered frame, keeping the retained observation window O(1) during a
  long-running camera session instead of failing after 1024 frames.
- [ ] Add a separately versioned raw-unary ABI and prove snapshot byte identity
  without base64 end to end.
