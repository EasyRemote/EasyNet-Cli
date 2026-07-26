# Intent

Goal: move daemon-to-FFI invocation error classification behind a daemon-owned typed projection so the C ABI adapter no longer inspects daemon error message text.

Non-goals:
- Change public C ABI, Go SDK, Python SDK, or product user-facing error codes.
- Add compatibility fallback paths for old data or legacy route wording.
- Rework remote routing semantics beyond the projection boundary.

Acceptance criteria:
- `src/ffi/invocation/mod.rs` delegates daemon error classification to a daemon-owned type.
- FFI production code no longer contains caller-signer or descriptor-owner-offline message classifiers.
- Existing caller signer and descriptor owner offline public projections remain byte-compatible.
- Convergence gates encode the new boundary.
