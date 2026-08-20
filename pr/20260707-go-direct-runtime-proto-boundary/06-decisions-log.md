# Decisions Log

## 2026-07-07

- Treat generated protobuf descriptor registration as a concrete transport
  side effect, not as part of the root Go SDK facade.
- Gate the direct daemon runtime with `easynet_direct_runtime` so default SDK
  consumers do not load private Axon protobuf descriptors.
- Keep backend protobuf deletion as a remaining SDK-only boundary requirement;
  this slice only removes accidental process-wide descriptor pollution from the
  SDK root import.
- Verified that the backend `internal/svc` package can now import the Go SDK
  while its temporary generated Axon protobuf package still exists. The final
  architecture still requires deleting that backend package rather than relying
  on coexistence.
