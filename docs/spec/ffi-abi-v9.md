# EasyNet Generic C ABI v9 Buffer-Lease Stream Extension

Status: additive extension to the frozen C ABI v7 base and v8 binary-frame
extension.

## Compatibility and discovery

- `RUNTIME_ABI_VERSION` remains `7`; v7 and v8 are unchanged.
- `include/easynet_cli.exports.v9` contains the complete v8 surface plus only
  `runtime_invocation_stream_open_v9`, `runtime_buffer_lease_retain_v9`, and
  `runtime_buffer_lease_release_v9`.
- A binding may use v9 only when all three symbols exist and
  `runtime_feature_discovery` reports
  `abi_extensions.v9.stream_buffer_lease == true`, the exact open/retain/release
  symbol names, and `symbols.stream_buffer_lease_v9 == true`.
- Malformed or incomplete discovery is not v9 capability. Such a binding may
  select the already specified v8 representation.
- Because the extension is additive, a valid pre-v9 discovery document may
  omit both `abi_extensions.v9` and `symbols.stream_buffer_lease_v9`. Presence
  of only part of the v9 tuple is invalid and MUST NOT enable v9.

This extension removes the mandatory payload copy between Runtime's decoded
server-stream frame and a native SDK consumer. It is not an end-to-end
zero-copy claim: protobuf/tonic decoding, TLS/kernel transport, and a language
API that materializes owned bytes may still copy.

The current Python and Go `StreamEvent` facades intentionally remain on v8:
their public payload is language-owned bytes and their event API has no
deterministic release operation. They MUST NOT select v9 until they expose an
explicit leased-event owner whose close/release remains coupled to every
escaped view. Silently wrapping a v9 pointer in the existing queued event would
either leak the bounded pool or create a dangling pointer.

## Frame and ownership contract

The normative declarations are `RuntimeBufferLeaseV9`,
`RuntimeInvocationStreamFrameV9`, `RuntimeInvocationStreamV9Callback`, and the
three v9 functions in `include/easynet_cli.h`.

The fixed header, kind/state values, flags, ordering, receipt verification,
terminal lifecycle, and exactly-one-null-frame EOF are identical to v8.
`payload_content_type`, admission receipt, terminal receipt, error, and the
frame struct remain borrowed for callback duration. Only `payload.data` has an
extended lifetime.

Every non-empty payload is `{lease_id != 0, data != NULL, len > 0}` and gives
the callback one owning reference. Empty payload is exactly `{0, NULL, 0}` and
must not be retained or released. The immutable payload pointer remains valid
until the lease's final successful release or shutdown of the creating
`RuntimeHandle`.

`runtime_buffer_lease_retain_v9` adds one reference;
`runtime_buffer_lease_release_v9` removes one. Both are thread-safe and require
the same live RuntimeHandle incarnation that opened the stream. Unknown,
already-released, stale, cross-handle, and reference-overflow operations fail
closed. A consumer that ignores a non-empty frame must still release its
initial reference.

## Bounds and closure

Each v9 stream admits at most 64 outstanding lease entries and at most 256 MiB
of queued-plus-delivered payload bytes. The reader acquires the byte permit
before callback-queue admission; that same permit moves into the lease entry
and is returned only by final release. Delivery waits when either bound would
be exceeded, propagating lossless backpressure through the bounded callback
queue and tonic reader. One RuntimeHandle may own at most 32 active server
streams and the process may own at most 256.

Explicit stream close and RuntimeHandle shutdown stop admission and wake a
blocked dispatcher. Natural EOF is emitted after queued frames drain; a
consumer holding the configured lease limit must release capacity before later
queued frames and EOF can be delivered. Already delivered leases remain valid
and releasable after stream close. RuntimeHandle shutdown revokes and frees all
of that session incarnation's remaining leases; consumers must not access their
pointers afterward. A payload larger than the byte bound produces an explicit
`PAYLOAD_TOO_LARGE` transport-error frame with an empty lease before EOF.

Close and RuntimeHandle shutdown request callback stop, cancel the reader,
wake lease admission, and wait for an in-flight callback before returning to
an external caller. Reentrant close/shutdown from the callback thread never
self-waits; it suppresses every later frame and the EOF callback for that
`user_data`.

Release archives contain the executable `libeasynet_cli` shared library as
well as the header, allowlist, and spec. Before signing and packaging, its
complete defined-global symbol table is reduced to and compared with the exact
v9 allowlist.

## Semantic and product boundary

The lease changes memory ownership only. Runtime Core remains the sole owner of
authority, sequence, state, admission, receipts, cancellation, error, timeout,
terminal, and EOF semantics. Lease ids are local capabilities and never enter
an Axon Invocation or receipt.

ABI v9 is for generic binary Ability server streams. RemoteApp WebRTC/shared
frames remain the interactive desktop audio/video data plane; v9 must not be
used as a second RemoteApp media tunnel.
