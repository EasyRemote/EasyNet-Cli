# Runtime Dispatch Latest Mode Boundary Proof

The runtime-dispatch UDS is a daemon-internal local-tool bridge, not the public daemon Invocation transport. Its request DTO still affects daemon/runtime architecture because implicit defaults create hidden compatibility paths and make request state ambiguous.

Requiring `mode` makes the request state explicit. Valid callers must select RPC or stream behavior intentionally. Missing or unknown mode is rejected with `BAD_REQUEST`, which aligns with the latest-only input rule and prevents stale caller behavior from becoming part of the canonical daemon contract.

The boundary guard now searches for both serde input aliases and runtime-dispatch fallback-mode vocabulary so legacy input paths cannot be reintroduced silently.
