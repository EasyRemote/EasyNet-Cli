# Boundary Proof

The selected seam is the Go SDK C ABI descriptor-resolution error boundary.
The Rust C ABI already records typed last-error JSON for descriptor resolver
failures. The SDK adapter must consume that typed slot as the canonical runtime
error, not reinterpret the numeric C ABI return code as a generic transport
failure or as legacy `NOT_FOUND` / `ABILITY_NOT_FOUND` vocabulary.

The converged boundary is:

1. C ABI `runtime_last_error_json` is authoritative when present.
2. `RuntimeClient.ResolveDescriptorRef` receives `DESCRIPTOR_NOT_FOUND` with
   `stage = routing` for descriptor misses.
3. Product callers never need to parse `"C ABI ... failed with code N"` to
   determine descriptor state.
4. Go and Python stay aligned: descriptor-resolution misses project as
   `DESCRIPTOR_NOT_FOUND` with routing stage, not ability-absence or generic
   C ABI compatibility errors.
