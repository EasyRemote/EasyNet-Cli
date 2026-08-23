# Execution checklist

- [x] Inspect exact v7/v8 exports, header and feature discovery.
- [x] Inspect Runtime callback metadata, payload ownership, EOF and errors.
- [x] Inspect Python binding selection, copying, queue bounds and fallback.
- [x] Inspect Go binding selection, copying, queue bounds and fallback.
- [x] Locate the actual RemoteApp/EasyRemote raw-stream consumer.
- [x] Run ABI, SDK and convergence mutation tests.
- [x] Fix only evidence-backed gaps; commit after staged-diff review.

Consumer finding: EasyRemote consumes typed raw payloads through the Python SDK
`RuntimeFrameStream` projection and therefore inherits v8 selection from the
C ABI provider. RemoteApp interactive media does not consume the server-stream
v8 entry point; its production data plane is WebRTC with binary InvokeBidi for
signaling/input. Treating lack of v8 use in RemoteApp as a defect would create a
second media tunnel instead of closing a product use case.
