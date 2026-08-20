# Python Control IPC Product-Call Boundary Plan

## Objective

Make the Python SDK control IPC client enforce the SPEC transport split:
`control.sock` is boot/status/discovery only and must not be usable as an
arbitrary product ability dispatch path.

## SPEC Anchor

- `docs/spec/daemon-sdk-requirements-v1.md#7.3` says `control.sock` is only for
  boot/status/discovery and product calls must use the daemon Invocation
  endpoint.
- `docs/json-control-caller-inventory.md` records the retained JSON control
  caller as daemon boot/status subscription via `system.watch_boot`.
- `src/daemon/control/frames.rs` and `src/daemon/control/server.rs` accept only
  `system.watch_boot` subscriptions plus cancel for that boot/status stream.

## Boundary Proof

Allowed:

- Read `control.json` discovery.
- Open a control IPC client for daemon boot/status observation.
- Subscribe to `system.watch_boot`.
- Cancel a boot/status subscription.

Forbidden:

- Sending product ability names such as `directory.subscribe`,
  `events.device.subscribe`, `consent.subscribe`, `invoke`, or `OpenBidi` over
  control IPC.
- Using `round_trip()` or generic `send()` to bypass the typed
  `subscribe()`/`cancel()` guardrails.
- Adding compatibility fallback paths that silently forward product calls over
  control frames.

## Implementation Steps

1. Add a small value-object style validator in Python `control_ipc.py` for
   outgoing control frames.
2. Route `send()`, `round_trip()`, `subscribe()`, and `cancel()` through that
   validator.
3. Add Python tests for accepted boot/status frames and rejected product
   control calls.
4. Extend the shared `daemon/control_only` conformance case expectations.
5. Run Python focused tests, Go conformance, scaffold/parity, and shared
   conformance runner.

## Verification

- `PYTHONPATH=tests uv run python -m unittest tests.test_control_ipc tests.test_conformance`
- `go test ./... -run 'Conformance|ImportBoundary'`
- `tools/scripts/check-sdk-parity-matrix.sh --self-test`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `cargo run --bin sdk-conformance-runner -- --language python --adapter-report sdk/conformance/runner/python-action-adapter-report.json`
