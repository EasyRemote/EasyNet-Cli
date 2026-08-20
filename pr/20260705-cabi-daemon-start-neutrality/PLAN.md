# C ABI Daemon Start Neutrality

## Objective

Align the native C ABI daemon start JSON and Go C ABI adapter with the daemon SDK `StartConfig` vocabulary from `docs/spec/daemon-sdk-requirements-v1.md`.

## Boundary Proof

- Ownership: C ABI projects the native daemon SDK lifecycle API; it must expose SDK terms, not product-era start-wire field names.
- Runtime delegation: Rust FFI still builds the existing daemon `DaemonStartConfig`; only the public JSON boundary changes from legacy fields to `device_id` and `detached`.
- Cross-language parity: Python and Go C ABI adapters now forward the same daemon start fields to the C ABI.
- Compatibility posture: legacy `node_id` and `detach` start fields are not accepted at the SDK boundary for this path, preventing dual public shapes.

## Implementation

- Parse `device_id` and `detached` in Rust FFI `easynet_daemon_start`.
- Project Go `StartConfig` C ABI JSON with `device_id` and `detached`.
- Update Rust and Go tests to reject legacy daemon-start field leakage.

## Verification

- Targeted Rust FFI daemon tests.
- Go SDK tests.
- Python SDK tests to ensure the Python C ABI adapter remains aligned.
- SDK scaffold, formatting, diff, and terminology scans.
