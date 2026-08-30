# EasyNet Generic C ABI v8 Binary Stream Extension

Status: current additive extension to the frozen C ABI v7 base.

## Compatibility and discovery

- The base ABI remains `RUNTIME_ABI_VERSION == 7` and its exact surface remains
  `include/easynet_cli.exports.v7`.
- The v8 extension surface is `include/easynet_cli.exports.v8`; it contains the
  full v7 surface plus only `runtime_invocation_stream_open_v8`.
- A binding may call the v8 symbol only when the dynamic symbol exists and
  `runtime_feature_discovery` reports all of:
  `abi_extensions.v8.stream_binary_frame == true`,
  `abi_extensions.v8.symbol == "runtime_invocation_stream_open_v8"`, and
  `symbols.stream_binary_frame_v8 == true`.
- If any condition is absent or malformed, the binding uses the frozen v7
  JSON/base64 stream representation.

The declarations live in `include/easynet_cli.h`. Python and Go perform one
required copy from callback-borrowed C memory into language-owned storage
before returning to the library. The Go binding transfers that owned storage
through its internal inbox and raw-event projection without cloning the
payload again; public accessors still return defensive copies.

This ABI is not the RemoteApp media data plane and is not a cross-language
zero-copy contract. The callback borrow ends when the callback returns, so an
SDK that queues a payload beyond that call must copy it into binding-owned
memory. v8 removes base64 and hot-path JSON from generic Runtime streams; it
does not replace the plugin-private shared-memory lease used for interactive
RemoteApp media.

Calling this surface an efficient end-to-end byte stream is incorrect. Tonic
still decodes protobuf `bytes` into an owned Rust buffer and a queued language
binding still needs the callback-boundary copy above. A future zero-copy
cross-language surface would need an explicit bounded frame lease with a
release operation; it cannot be simulated by retaining this callback pointer.

## Callback contract

The normative declarations are `RuntimeBytesViewV8`,
`RuntimeInvocationStreamFrameV8`, the `RUNTIME_STREAM_FRAME_V8_*` constants,
and `RuntimeInvocationStreamV8Callback` in `include/easynet_cli.h`.

The callback receives one borrowed, fixed-layout frame pointer. `kind`,
`state`, `sequence`, `elapsed_ms`, `terminal`, and `transport_terminal` are
represented by scalar fields and flags. `payload_content_type` and `payload`
are length-delimited byte views. Normal data frames therefore perform no JSON
serialization or parsing in the C ABI hot path.

`admission_receipt_json`, `terminal_receipt_json`, and `error_json` are sparse,
length-delimited JSON object sidecars. Their pointers are null and lengths are
zero when absent; their matching presence flag MUST agree with the view. They
exist only on the lifecycle frames that carry those values and never duplicate
the payload.

Every frame MUST report `abi_version == 8`, a `struct_size` at least as large as
the published layout, a positive sequence, known kind/state values, and no
unknown flag bits. Bindings MUST reject rather than repair malformed or
regressing frames.

Bindings MUST reject, not repair, every malformed binary frame.

EOF is exactly one callback with `frame == NULL`. EOF is transport closure, not
an inferred Invocation terminal state.

A binding-local queue overflow or malformed callback tuple is a carrier error,
not a Runtime frame. Bindings MUST surface it through their typed SDK error path
and MUST NOT invent a Runtime `sequence`, lifecycle state, receipt, or terminal
frame to represent it.

## Ownership and product use

Binary framing and raw bytes are transport representations, not a second Invocation model.
Runtime Core remains the sole owner of authority validation, receipt
verification, ordering, cancellation, terminal lifecycle, and bounded
backpressure. SDKs project the raw packet into the same public stream state
machine used by v7.

EasyRemote server streams may consume this extension for typed binary payloads
whose callback-borrow/copy cost is acceptable. RemoteApp WebRTC remains the
interactive audio/video transport; encoded H.264/Opus moves from its private
media host through bounded shared-memory slot leases and then RTP/SRTP. Its
`metadata_json_plus_binary` InvokeBidi signaling and input path is not replaced
by this server-stream ABI. The two paths share canonical Invocation authority
and lifecycle semantics but do not tunnel media through one another.
