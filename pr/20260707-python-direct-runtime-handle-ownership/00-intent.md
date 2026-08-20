# Intent

Converge the Python direct daemon runtime with the canonical SDK runtime model by making handle-transport ownership explicit.

The direct daemon gRPC transport owns direct invocation, stream, and bidi calls. Prepared invocation, signed submission, and handle lifecycle operations remain SDK-owned handle-transport responsibilities. The Python SDK must model that split with the same ownership semantics as the Go SDK instead of relying on an implicit non-owning delegate.

