# FFI raw stream v8 plan

## Problem

EasyRemote and RemoteApp need high-frequency binary data planes for desktop,
window, application, audio, and video streams. ABI v7 currently projects stream
payloads through canonical JSON frames and base64, which is compatible but not a
long-term transport representation for large or frequent payloads.

## Architecture invariant

Raw bytes are an ABI v8 transport representation only. Invocation authority,
sequence, admission receipt, terminal receipt, error shape, terminal lifecycle,
and state-machine ownership remain Runtime Core responsibilities.

## Boundary rules

- ABI v7 remains frozen and continues to expose JSON/base64 stream frames.
- ABI v8 is additive and discoverable; it must not change `RUNTIME_ABI_VERSION`.
- ABI v8 callbacks split metadata JSON from raw payload bytes.
- Metadata JSON must continue to carry canonical frame lifecycle fields.
- SDK facades may select v8 when available and must fall back to v7.
- Direct runtime transports stay canonical JSON; raw packets are a C ABI v8
  provider concern.

## Implementation checklist

- Add `RuntimeInvocationStreamV8Callback` and
  `runtime_invocation_stream_open_v8` to `include/easynet_cli.h`.
- Add `include/easynet_cli.exports.v8` while preserving the exact v7 allowlist.
- Extend feature discovery with `abi_extensions.v8.stream_raw_payload`.
- Implement Rust FFI v8 callback delivery with metadata JSON plus raw bytes.
- Keep v7 stream delivery as canonical JSON with base64 projection.
- Add Python C ABI binding support for v8 raw packets and v7 fallback.
- Add tests proving v8 preserves raw payload bytes and does not duplicate
  payload fields into metadata JSON.
- Add SDK/provider tests proving v8 raw metadata carries the full canonical
  lifecycle contract and fails closed when required fields are omitted.
- Add gates that lock v7 compatibility and v8 extension discipline.

## Verification plan

- `cargo fmt --all`
- `python -m py_compile sdk/python/easynet_sdk/_cabi.py sdk/python/easynet_sdk/stream.py sdk/python/tests/test_cabi.py`
- `python -m pytest sdk/python/tests/test_cabi.py -q`
- `cargo test -p easynet stream_v8_delivery_preserves_raw_payload_without_json_projection --lib`
- `cargo test -p easynet stream_delivery_preserves_non_json_payload_as_base64_projection --lib`
- `bash tools/scripts/check-ffi-abi-v7-header.sh`
- `bash tools/scripts/check-ffi-abi-v8-header.sh`
- Canonical SDK/runtime convergence gates after the targeted ABI path is green.
