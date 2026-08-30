# v8 raw stream metadata contract

## Invariant

Raw stream payload bytes are an ABI v8 transport representation only. They must
not weaken Runtime Core stream lifecycle semantics. Every v8 metadata frame must
carry the canonical lifecycle and receipt fields needed by SDK state machines:
`sequence`, `kind`, `state`, `terminal`, `transport_terminal`,
`payload_content_type`, `admission_receipt`, `terminal_receipt`, and `error`.

## Change

- Keep ABI v7 JSON/base64 stream parsing compatible.
- Make Python SDK `RawStreamPacket` parsing fail closed when v8 metadata omits
  canonical lifecycle fields.
- Keep `payload_base64` and `payload_json` forbidden in v8 metadata; the raw
  bytes remain the only payload representation.
- Make Rust FFI transport-error metadata include the same canonical v8 fields.
- Add targeted Python/Rust tests for the contract.

## Product effect

EasyRemote/RemoteApp can use raw bytes for high-frequency media without letting
binary transport bypass sequence, lifecycle, receipt, error, or terminal state
machine semantics.
