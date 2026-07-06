# Architecture

## Current Defect

Python `profile_bridge.py` has a private `_gateway_status_json` projector that accepts raw or partially legacy response shapes and derives `process_live`, `control_ready`, `runtime_ready`, `directory_ready`, `trust_ready`, `public_listener_ready`, and `ready`.

That duplicates the native Admin + Gateway projection state machine in `src/protocol/admin_gateway_contract.rs` and diverges from Go, where gateway status comes from an explicit provider and is validated as `GatewayStatus`.

## Target Shape

- Native Rust/C ABI projection remains the owner of daemon lifecycle to `GatewayStatus` conversion.
- Go continues to validate canonical `GatewayStatus` JSON.
- Python profile bridge becomes a facade over canonical `GatewayStatus` DTO output for `gateway.status`.
- Non-canonical profile bridge dispatcher output fails closed before it can be exposed as SDK status.

## Boundary Proof

The Python profile bridge may build complete Invocation carriers and call a narrow `ProfileBridgeDispatcher`, but it must not derive daemon readiness policy. Readiness depends on daemon lifecycle, invocation endpoint, directory/session admission, trust, and public listener requirements. Those are SDK core / daemon-owned semantics, not Python facade ergonomics.
