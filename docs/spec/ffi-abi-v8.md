# EasyNet Generic C ABI v8 Raw Stream Extension

Status: current additive extension to the frozen C ABI v7 base.

## Compatibility and discovery

- The base ABI remains `RUNTIME_ABI_VERSION == 7` and its exact surface remains
  `include/easynet_cli.exports.v7`.
- The v8 extension surface is `include/easynet_cli.exports.v8`; it contains the
  full v7 surface plus only `runtime_invocation_stream_open_v8`.
- A binding may call the v8 symbol only when the dynamic symbol exists and
  `runtime_feature_discovery` reports all of:
  `abi_extensions.v8.stream_raw_payload == true`,
  `abi_extensions.v8.symbol == "runtime_invocation_stream_open_v8"`, and
  `symbols.stream_raw_payload_v8 == true`.
- If any condition is absent or malformed, the binding uses the frozen v7
  JSON/base64 stream representation.

The declarations live in `include/easynet_cli.h`. Python and Go copy borrowed
callback memory before returning to the library.

## Callback contract

```c
typedef void (*RuntimeInvocationStreamV8Callback)(
    void *user_data,
    const char *metadata_json,
    const uint8_t *payload,
    size_t payload_len
);
```

`metadata_json` is the canonical Runtime Core frame and contains `sequence`,
`kind`, `state`, `terminal`, `transport_terminal`, `payload_content_type`,
`admission_receipt`, `terminal_receipt`, and `error`. It must not contain a
second `payload_base64` or `payload_json` representation. `payload` contains the
exact bytes and may be empty for data, terminal, or error frames.

EOF is exactly one all-null callback: `metadata_json == NULL`, `payload ==
NULL`, and `payload_len == 0`. EOF is transport closure, not an inferred
Invocation terminal state.

## Ownership and product use

Raw bytes are a transport representation, not a second Invocation model.
Runtime Core remains the sole owner of authority validation, receipt
verification, ordering, cancellation, terminal lifecycle, and bounded
backpressure. SDKs project the raw packet into the same public stream state
machine used by v7.

EasyRemote server streams may consume this extension for typed media and large
binary payloads. RemoteApp WebRTC remains the interactive audio/video transport;
its `metadata_json_plus_binary` InvokeBidi signaling and input path is not
replaced by this server-stream ABI. The two paths share canonical Invocation
authority and lifecycle semantics but do not tunnel media through one another.
