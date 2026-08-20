# Python pluginexec nonce byte parity

## Goal

Align the Python runtime provider sidecar helper with the canonical sidecar
frame model implemented by the Go, Node, Java, and Rust provider helpers.

## Root abstraction problem

`invocation_nonce` is a canonical 16-byte JSON numeric vector. Python's `bool`
is a subclass of `int`, so the current Python helper admits `True` and `False`
as nonce bytes even though the other language helpers reject booleans as
non-numeric JSON byte values. That creates a language-specific acceptance path
for non-canonical sidecar frames.

## Architecture decision

- Keep pluginexec under the provider namespace; do not expose product-specific
  plugin abstractions from the root SDK.
- Treat nonce validation as part of the canonical sidecar frame projection.
- Reject Python `bool` explicitly before accepting integer byte values.
- Add conformance gate coverage so this cannot regress silently.

## Files

- `sdk/python/easynet_sdk/providers/runtime/plugin_exec.py`
- `sdk/python/tests/test_plugin_exec.py`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`

## Verification

- Python focused pluginexec test
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-sdk-product-neutrality.sh`
- `cargo fmt --check`
- `git diff --check`
- codegraph query for pluginexec nonce parity
