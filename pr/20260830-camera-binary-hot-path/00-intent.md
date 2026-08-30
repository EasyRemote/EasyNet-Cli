# Intent

## Goal

Make camera preview, still capture, and recording use the lowest-complexity
product path that the current Runtime contract supports: latest-frame native
capture, typed raw unary/stream payloads, and constant-memory recording
persistence.

## Acceptance

- Camera preview emits `image/jpeg` as Runtime payload bytes without daemon
  JSON/base64 projection.
- `camera.snapshot` returns the native JPEG as an RPC byte result with exact
  `image/jpeg` content type; it never allocates a base64 representation.
- A slow consumer retains at most one unpublished camera frame.
- Recording stop never reads the complete movie into process memory.
- A successful stop receipt names an already-durable Context artifact.
- Invocation sequence, receipts, cancellation, and terminal closure remain
  owned by Runtime Core.

## Non-goals

- Do not redefine Axon wire semantics; protobuf unary results and ABI v8
  streams already carry raw payloads.
- Do not modify concurrent RemoteApp files.
- Do not claim zero-copy: AVFoundation, JPEG encoding, gRPC, and SDK boundaries
  necessarily own bounded copies.
