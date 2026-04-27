# AXON-RFC-003 — InvokeBidi Data-Plane Design Review

**Status**: design review, no code yet. Awaiting approval before
P0 implementation.
**Date**: 2026-04-27
**Author**: Claude (under Silan.Hu architectural authority)
**Scope**: task C-M1b — finalise the data-plane RPC that
complements the existing Invoke + InvokeStream control plane,
with a stable FFI suitable for Go/Node/Python SDK consumption.

---

## Why this RFC exists now

The control plane (Invoke + InvokeStream) is complete and
validated:
- `Invoke` carries unary AXIOM-7-tuple-signed calls.
- `InvokeStream` server-streams chunked receipts (used by
  `federation.subscribe_directory` and the SSE broker as of
  C-M11/C-M12).
- The aggregation layer (backend-profile Agent + `aggregate.*`)
  rides entirely on these two RPCs; backend has zero need for
  bidi.

What's NOT yet ratified:
1. The InvokeBidi proto exists and the Axon kernel implements
   it (P5-rewrite-15 landed the wire, RFC-002 Stage 1 made it
   dispatch through `SessionRegistry`). But the **public FFI**
   that Go/Node/Python SDKs consume is unspecified — today only
   the Rust-internal `BidiStreamHandle` exists.
2. **Multimodal semantics** — current `BinaryChunk + StreamDescriptor`
   handles single-modality (PTY) cleanly; lip-sync / multi-track
   media (audio + video + subtitle) and frame metadata
   (`pts`, codec params, key-frame markers) are sketched in the
   proto but unspecified.
3. **args_root_hash vs args_digest** — for streamed initial args
   that exceed a single frame, the AXIOM signing semantics need
   a decision.
4. **Backpressure** — the Rust kernel uses bounded mpsc; the
   wire / FFI semantics need to match HTTP/2 flow control so
   slow consumers don't memory-bomb the producer.

This RFC settles those four gaps without changing the existing
RPC signature or breaking RFC-002 Stage 1.

---

## §1 — RPC naming and separation from InvokeStream

**Decision**: keep `InvokeBidi` as a distinct RPC. Do not merge
with `InvokeStream`.

**Wire signature** (already shipped, do not modify):
```proto
service Axon {
  rpc Invoke(InvokeRequest) returns (InvokeResponse);
  rpc InvokeStream(InvokeServerStreamRequest)
      returns (stream InvokeStreamChunk);
  rpc InvokeBidi(stream InvokeBidiUp)
      returns (stream InvokeBidiDown);
}
```

**Rationale for separation**:

1. **Admission timing differs.** `InvokeStream` runs admission
   once at the request message, before any chunk flows. `InvokeBidi`
   runs admission once at frame 0 (the signed `EnvelopeOpen`),
   and the rest of the stream is integrity-bound by the HMAC
   chain. Collapsing them would force `InvokeStream` to also
   carry an Ed25519-signed frame 0, which it doesn't need (the
   request message IS the signed envelope).

2. **Symmetry of the data plane.** `InvokeBidi` is the only RPC
   where the caller pushes data after admission. SSE-style
   server-stream pushes from server only. Forcing a single RPC
   would require a sentinel "this stream has caller frames" flag
   that subdivides the schema, which is exactly the kind of
   "silent semantic upgrade" the strict-separation directive
   forbids.

3. **Frame integrity model differs.** `InvokeStreamChunk`
   sequence numbers are advisory (chunks ride the unary signed
   envelope's trust). `InvokeBidiUp/Down` chain MACs link every
   frame to the prior one + the envelope signature, so a
   tampered mid-stream chunk is detectable. Two different
   models, one RPC each.

4. **Old SDK negative behaviour.** Distinct method names mean a
   pre-RFC SDK calling the wrong RPC gets `UNIMPLEMENTED`, not a
   silent "looks like it worked but the wire doesn't match what
   the caller signed."

**Naming convention going forward**:
- `Invoke` = unary, signed, blocking response.
- `InvokeStream` = server-stream, signed-once, advisory chunks.
- `InvokeBidi` = bidirectional-stream, signed-frame-0 +
  HMAC-chained subsequent frames.

The names are stable. Future modes (e.g. a hypothetical
`InvokeChannel` for long-lived multi-call sessions) get NEW RPC
names, never overload existing ones.

---

## §2 — Frame schema (BinaryChunk, Control, Receipt)

**Decision**: keep the existing schema. Add three documented
extensions to `BinaryChunk` and one to `BidiControl` for
multimodal media. No new top-level frame variants.

### §2.1 — Existing schema (unchanged)

```proto
message InvokeBidiUp {
  uint64 sequence = 1;
  bytes  mac      = 2;
  oneof payload {
    EnvelopeOpen envelope_open = 10;  // sequence MUST be 0
    BinaryChunk  binary_chunk  = 11;
    BidiControl  control       = 12;
  }
}

message InvokeBidiDown {
  uint64 sequence = 1;
  bytes  mac      = 2;
  oneof payload {
    InvocationReceipt receipt      = 10;  // frame 0 = admission
    BinaryChunk       binary_chunk = 11;
    BidiControl       control      = 12;
  }
}
```

**Frame role contract** (final):

| Role | Up frame 0 | Up frame N≥1 | Down frame 0 | Down frame N≥1 |
|---|---|---|---|---|
| `EnvelopeOpen` | REQUIRED | forbidden | forbidden | forbidden |
| `InvocationReceipt` | forbidden | forbidden | REQUIRED (admission accept) | optional (interim/terminal state) |
| `BinaryChunk` | forbidden | allowed | forbidden | allowed |
| `BidiControl` | forbidden | allowed | forbidden | allowed |

A receiver MUST reject any violation (already implemented in
`bidi_handler.rs`).

### §2.2 — BinaryChunk additions for multimodal

`BinaryChunk` today carries `stream_id`, `data`, `pts`. Three
additions are needed for production multimodal:

```proto
message BinaryChunk {
  uint32 stream_id = 1;
  bytes  data      = 2;
  uint64 pts       = 3;

  // NEW (RFC-003):
  bool   key_frame = 4;   // §2.2.a
  uint32 duration  = 5;   // §2.2.b — microseconds
  uint64 dts       = 6;   // §2.2.c
}
```

**§2.2.a — `key_frame`**: video codecs (h264, av1, vp9) require
the receiver to know which frames are independently decodable
(I-frames vs P/B-frames). Without this flag, a receiver joining
mid-stream cannot decode until the next keyframe — and has no
way to know when that is. PTY and audio set `key_frame = true`
for every frame (they're always independently decodable);
field default of `false` is wrong for non-video codecs but
costs only one byte per frame.

**§2.2.b — `duration`**: media chunks have known durations
(20ms for Opus packets, 33ms for 30fps video). Receivers use
this for jitter buffer sizing and lip-sync. Optional (zero =
"unknown, treat as instantaneous").

**§2.2.c — `dts`**: Decode Timestamp. For codecs with B-frames
(h264, hevc), DTS != PTS. v1 single-modality (PTY) ignores
this; v1 audio also ignores it (audio has DTS == PTS); v1 video
producers MAY emit it. v1 receivers SHOULD pass it through to
the decoder.

These three fields are **additive and optional** — every existing
v1 receiver continues to work because they default to zero/false
and the existing PTY round-trip never reads them.

### §2.3 — BidiControl additions

```proto
message BidiControl {
  oneof control {
    PtyResize       pty_resize  = 1;
    PtySignal       pty_signal  = 2;
    MediaTimestamp  media_pts   = 3;
    bool            eof         = 4;

    // NEW (RFC-003):
    StreamReady     stream_ready = 5;   // §2.3.a
  }
}

// §2.3.a — Sender signals "I am about to start sending on
// stream_id; here is the first PTS." Allows receivers to
// pre-allocate jitter buffers and synchronise multi-stream
// reference clocks. Optional; receivers tolerate streams that
// start without it.
message StreamReady {
  uint32 stream_id      = 1;
  uint64 first_pts      = 2;
  uint64 timeline_unix_ms = 3;  // wall-clock anchor for the
                                // stream's reference clock
}
```

No removal, no rename. v1 receivers ignore unknown
`BidiControl.control` variants per protobuf default behaviour.

### §2.4 — Receipt placement

The existing rule (down-frame-0 = admission `InvocationReceipt`,
final down-frame = terminal `InvocationReceipt`) is final.
Interim receipts (state = `Running`) MAY appear between binary
chunks for long-lived sessions; consumers parse them as "session
still alive, no terminal yet." This matches the
`InvokeStreamChunk` pattern.

---

## §3 — args_root_hash vs args_digest decision

**Decision**: keep `args_digest` (single SHA-256 of the
canonical args bytes). Do NOT introduce `args_root_hash`
(Merkle-tree root over chunked args).

**Context**: `EnvelopeOpen.initial_args` is a `bytes` field. In
principle, `initial_args` could be:
(a) the entire arguments payload (single frame), OR
(b) a manifest pointing to chunks streamed in subsequent
    BinaryChunk frames before the ability handler runs.

(b) would need `args_root_hash` — a Merkle tree over the chunks
so the signed envelope commits to all of them without putting
multi-MB args inline. (a) needs only `args_digest` = SHA-256 of
the inline bytes.

**Rationale for (a) — keep `args_digest`**:

1. **No production use case for streamed args.** Every InvokeBidi
   ability today (PTY attach, future LLM session, future MCP
   bridge) opens with small args (`{"session_id": "..."}`,
   ~30 bytes) and then streams ability-specific data via
   BinaryChunk. The ability **input** is small; the ability
   **output / interaction** is the streamed part. There is no
   ability where the input itself is multi-MB and needs chunking.

2. **gRPC frame size limits make (a) safe in practice.** gRPC's
   default max message size is 4 MiB. `EnvelopeOpen.initial_args`
   plus the rest of frame 0 fits comfortably under that for any
   realistic ability. If a future ability genuinely needs >4 MiB
   args, the right fix is to upload them via the existing
   `transfer.proto` (PayloadStore) and pass a payload reference
   in `initial_args` — not to invent a Merkle-tree mode.

3. **Merkle trees are a lot of complexity for a hypothetical.**
   `args_root_hash` would require:
   - A canonical chunk-size declaration.
   - A tree-shape declaration (binary vs Patricia).
   - A receiver-side accumulator that buffers chunks before
     dispatching the ability.
   - SDK code in five languages to build the tree client-side.
   None of this is justified by a use case we have today.

4. **Forward compatibility is preserved.** If a future RFC
   genuinely needs streamed args, it can:
   - Add a NEW field `EnvelopeOpen.args_root_hash bytes = 7;`
   - Specify that when this field is non-empty, `initial_args`
     is interpreted as the manifest and subsequent chunks on a
     reserved `stream_id = 0` carry the args body.
   v1 receivers ignore unknown fields; the addition is
   non-breaking.

**Concrete contract**:
- `EnvelopeOpen.initial_args` is the FULL canonical-encoded
  args byte string.
- `args_digest` (used in `canonical_invocation_bytes` for the
  Ed25519 signature) is `SHA-256(initial_args)`.
- Receivers compute `args_digest` from the received
  `initial_args` bytes and verify against the signed envelope.
- Receivers feed `initial_args` into the ability handler
  verbatim.

Pin this in §A8 of the existing AXIOM checklist.

---

## §4 — Canonical streaming signature anchor

**Decision**: keep the existing single-Ed25519-anchor model.
Frame 0's mac is the signature over `canonical_invocation_bytes`;
all subsequent frame MACs chain back to that signature via HKDF.

**Wire formula** (from current `bidi_handler.rs`, do not change):

```
frame 0 up.mac    = Ed25519_sign(caller_priv, canonical_invocation_bytes)
                    = envelope.caller_signature.signature
                    (the same 64 bytes are placed in BOTH locations
                    — this closes the downgrade gap where a caller
                    could sign canonical bytes with key A but anchor
                    the chain with a different signature B from the
                    same key)

up_key            = HKDF-SHA256(
                      salt = "easynet-bidi-mac:v1",
                      ikm  = envelope.caller_signature.signature || envelope.nonce,
                      info = "caller-to-callee",
                      L    = 32)

down_key          = HKDF-SHA256(
                      salt = "easynet-bidi-mac:v1",
                      ikm  = envelope.caller_signature.signature || envelope.nonce,
                      info = "callee-to-caller",
                      L    = 32)

frame N≥1 up.mac  = HMAC-SHA256-32(
                      up_key,
                      sequence_be_8 || prev_up_mac || canonical_payload_bytes)

frame N≥1 down.mac= HMAC-SHA256-32(
                      down_key,
                      sequence_be_8 || prev_down_mac || canonical_payload_bytes)
```

where `canonical_payload_bytes` is the deterministic protobuf
encoding of the same frame with the `mac` field zeroed out (so
the mac self-cover does not recurse).

**Rationale**:

1. **Symmetry with unary Invoke.** Frame 0 is structurally
   identical to a unary InvokeRequest envelope — same bytes,
   same signature, same admission code path. One canonical
   encoder, one verifier.

2. **No additional crypto material needed in steady state.**
   After admission, the session needs only the 32-byte derived
   keys. Re-signing every frame with Ed25519 would be 100×
   slower at video frame rates.

3. **HMAC chain provides integrity + ordering + anti-replay
   together.** Re-ordering or duplicating any frame breaks the
   chain at the next frame's MAC verify. This is stronger than
   what TLS gives (TLS guarantees integrity within a record but
   not across reconnects); for a long-lived bidi session
   spanning a TLS resumption, the HMAC chain still detects
   tampering.

4. **Per-direction keys via HKDF info label.** Without separate
   info labels an attacker who replayed an up frame as a down
   frame (in a deployment that proxied both directions through
   the same memory) could potentially replay-attack across
   directions. Distinct `info` labels make up and down MACs
   non-interchangeable even with the same `ikm`.

5. **Salt versioning.** The salt string `"easynet-bidi-mac:v1"`
   carries a version suffix. A future crypto change (move to
   BLAKE3, rotate to per-session-secret derivation) bumps to
   `:v2`; v1 receivers reject `:v2` envelopes at admission.

**No changes proposed.** This section locks the existing model
as the v1 signature anchor — it's already correct.

---

## §5 — HMAC frame-chain model

**Decision**: keep the existing chain shape. Add explicit
sequence-window enforcement and document the `mark_failed`
semantics.

### §5.1 — Sequence rules (final)

- Up sequence and down sequence are **independent counters**,
  each starting at 0 and incrementing by 1 per frame in their
  own direction.
- Up frame 0 MUST have `sequence = 0` AND `payload =
  EnvelopeOpen` (already enforced).
- Down frame 0 MUST have `sequence = 0` AND `payload =
  InvocationReceipt(state ∈ {accepted, ...})`.
- Frame N≥1 in either direction MUST have `sequence ==
  last_seen_in_that_direction + 1`. Out-of-order frames =
  immediate stream close with `AXON_BIDI_FRAME_SEQUENCE`.
- A receiver MUST drop any frame whose sequence re-uses a
  previously-seen value within the session
  (`AXON_BIDI_FRAME_REPLAY` — new code, not yet implemented;
  current code only catches the "skip ahead" case).

### §5.2 — Per-direction key recap

```
up_key    : caller writes / callee verifies
down_key  : callee writes / caller verifies
```

A caller computing a "down" MAC and sending it on the up
direction would fail verification at the callee because the
HMAC was computed with the wrong key. This is the property the
distinct HKDF info labels buy.

### §5.3 — Chain-break failure mode

When MAC verification fails on any frame:
1. The receiver immediately ceases reading further frames in
   that direction.
2. Emits a terminal `InvocationReceipt(state = Failed,
   reason = AXON_BIDI_FRAME_MAC_INVALID)` on the down direction
   (or, if the failure is on the down direction, surfaces it as
   the call's terminal status).
3. Closes the gRPC stream cleanly (no half-open state).

`mark_failed` (the `BidiStreamHandle` method that providers
call) is the kernel-side analogue: a provider observing a
backend-level failure annotates the failure reason via
`mark_failed("...")` then drops the handle. The kernel reads
the slot after the down channel closes and emits the terminal
with that reason. This is unchanged from RFC-002 Stage 1.

### §5.4 — What the chain does NOT cover

- **Confidentiality.** TLS is the confidentiality layer;
  the HMAC chain is integrity + ordering + anti-replay only.
  A future amendment can add per-direction AEAD (ChaCha20-Poly1305
  with the same HKDF-derived keys) for a defence-in-depth
  posture against compromised TLS terminators. Not in v1.
- **Liveness.** A silent peer (TCP open, no frames flowing)
  triggers no chain-side error. Liveness is a transport
  concern: gRPC keepalive pings + per-side timeouts. Receivers
  MAY emit `BidiControl.eof = true` on their own
  inactivity-timeout policy; receivers MUST NOT use the chain
  to enforce activity.

---

## §6 — FFI raw-byte handle API (Send/Recv)

**Decision**: define a stable C ABI exposing only **raw bytes,
control variants, and lifecycle signals** — never proto types,
never sequence/MAC visibility. SDKs in Go/Node/Python/Java/Swift
import the C ABI via cgo / N-API / ctypes / JNI / Swift C
interop.

### §6.1 — Why raw-byte FFI, not proto-typed

1. **The kernel owns frame integrity.** SDKs that touch
   sequence / MAC / canonical encoding can drift from the
   kernel's encoder, breaking interop subtly. The Rust kernel
   produces canonical bytes once; SDKs feed bytes in/out and
   never see the chain math.
2. **Codec-blindness.** The FFI doesn't know what a BinaryChunk
   carries. PTY, Opus, h264 — all the same `(stream_id,
   bytes, optional_pts)` triple at the FFI layer. The
   encoder/decoder lives in the SDK consumer or the ability
   handler, not in axon.
3. **Cross-language ABI stability.** A C ABI with primitive
   types (no proto) gives every SDK identical behaviour. The
   minute we expose protobuf types across the FFI we tie SDKs
   to a specific protoc-gen version per language.

### §6.2 — Handle lifecycle

```
open    : easynet_bidi_open(envelope_bytes, envelope_len,
                            handle_out)
            → status_code

send_chunk : easynet_bidi_send_chunk(handle, stream_id,
                                     data, data_len,
                                     pts_or_zero,
                                     flags)
              → status_code

send_control : easynet_bidi_send_control(handle, control_type,
                                         payload, payload_len)
                → status_code
              (control_type ∈ {pty_resize, pty_signal,
                               media_pts, eof, stream_ready})

recv_next : easynet_bidi_recv_next(handle,
                                   timeout_ms,
                                   frame_kind_out,
                                   stream_id_out,
                                   data_buf, data_buf_len,
                                   data_len_out,
                                   pts_out)
              → status_code
            (frame_kind ∈ {binary_chunk, control, receipt,
                           channel_closed})

mark_failed : easynet_bidi_mark_failed(handle,
                                        reason, reason_len)
                → status_code

close   : easynet_bidi_close(handle) → status_code
```

### §6.3 — Status codes

```
0  = OK
1  = WOULD_BLOCK     (non-blocking call, no frame ready;
                      retry after timeout or use blocking variant)
2  = CHANNEL_CLOSED  (stream terminal observed; further calls
                      are programmer errors)
3  = INVALID_ARG     (e.g. data_len > max chunk size)
4  = HANDLE_INVALID  (use-after-close)
5  = ENVELOPE_INVALID(open-time only — caller signature failed
                      verification, etc.)
6  = INTERNAL        (kernel bug; logged on the kernel side)
```

SDKs map these to language-idiomatic exceptions. There is no
"partial frame" status — the kernel either delivers a complete
frame or reports CHANNEL_CLOSED.

### §6.4 — What the FFI does NOT expose

- Sequence numbers (kernel-private)
- MAC bytes (kernel-private)
- HKDF keys (kernel-private)
- Raw protobuf bytes (SDKs build envelope_bytes via the
  per-language protoc-gen; that's the only proto contact)

### §6.5 — Existing Rust internal handle (unchanged)

`BidiStreamHandle` from RFC-002 Stage 1 stays as the kernel-
internal type. The C ABI is a thin wrapper that:
- on `open`: calls into the kernel's `bidi_handler::handle_invoke_bidi`
  pump + grabs the resulting `BidiStreamHandle`.
- on `send/recv`: routes through the existing
  `BidiStreamWriter` / `BidiStreamReader` halves.
- on `mark_failed`: calls the existing `mark_failed` method.

**No changes to RFC-002 Stage 1 internals.** The FFI is purely
additive.

### §6.6 — SDK consumption pattern

```
// pseudo-code, every language
handle = client.OpenBidi(envelope)
go {
  for {
    frame = handle.RecvNext()
    if frame.IsClosed { break }
    if frame.IsBinaryChunk {
      consumer.OnChunk(frame.StreamID, frame.Data, frame.PTS)
    } else if frame.IsControl {
      consumer.OnControl(frame.ControlType, frame.Payload)
    } else if frame.IsReceipt {
      consumer.OnReceipt(frame.Receipt)
    }
  }
}
producer.OnReady = func(stream_id, data, pts) {
  handle.SendChunk(stream_id, data, pts)
}
```

Same pattern in Go (channels), Node (async iterators), Python
(async for). The FFI is the same; the language idiom wraps it.

---

## §7 — Backpressure semantics (gRPC flow control)

**Decision**: rely on HTTP/2 flow control end-to-end. The
kernel's bounded mpsc (32 frames per direction) is the back-
stop, NOT the primary backpressure mechanism.

### §7.1 — How it works

1. **gRPC HTTP/2 flow control** is the wire-level back-pressure.
   The HTTP/2 receiver advertises a window (default 64 KiB);
   the sender stalls when the window is exhausted. tonic
   (Rust server) and grpc-go (Go client) both implement this
   correctly out of the box.

2. **Kernel bounded mpsc** (`BIDI_HANDLE_CHANNEL_DEPTH = 32`,
   shipped in RFC-002 Stage 1) absorbs short bursts past the
   HTTP/2 window. When this fills:
   - `send_chunk(...).await` blocks (Rust async backpressure).
   - The provider stops producing.
   - The HTTP/2 sender naturally stops too because nothing's
     being read off its socket.

3. **FFI `send_chunk` is blocking by default.** When the
   downstream is congested, the C ABI blocks the calling
   thread. SDKs that need non-blocking semantics either:
   - call `recv_next` with timeout=0 and treat WOULD_BLOCK as
     "buffer full, drop or retry";
   - run the FFI on a dedicated goroutine / thread (the
     idiomatic pattern).

### §7.2 — What this means per stream type

| Stream type | Behaviour when receiver is slow |
|---|---|
| PTY (text) | Sender blocks; user types and waits. Right answer — typing characters that get dropped silently is worse than a momentary lag. |
| Audio | Sender blocks; producer SHOULD drop the packet rather than block (codec-specific decision the SDK consumer makes). FFI gives the option via `flags` (§6.2). |
| Video | Same as audio. |
| Bulk file transfer | Sender blocks; this is the desired behaviour. |

The kernel does NOT make codec-specific drop decisions. The
SDK consumer (the audio app, the video app) chooses drop-or-
block per frame.

### §7.3 — The `flags` field on send_chunk

```
flags bits:
  0x01 = NONBLOCKING — return WOULD_BLOCK instead of blocking
                       when the kernel's mpsc is full
  0x02 = LOSSY      — kernel may drop this frame if the chain
                       can absorb the loss (RESERVED v2 — for
                       LOSS_TOLERANT streams; v1 ignores)
  0x04..0x80 = RESERVED
```

v1 callers should only set `0x01` when they need non-blocking
semantics; v1 receivers ignore `0x02` and behave as if
strict-ordered (which they are — every v1 stream is STRICT per
the existing `StreamDescriptor.ordering` field).

### §7.4 — Slow-receiver detection

A receiver that stalls for >`BIDI_RECEIVER_STALL_THRESHOLD_MS`
(propose 30s) without acking a frame triggers the kernel to
emit `BidiControl.eof` on the up direction and close the down
direction with `state = Failed, reason = AXON_BIDI_RECEIVER_STALL`.
This prevents a half-alive client from holding kernel goroutines
indefinitely.

Detection is purely transport-level (HTTP/2 PING + ack
absence). No frame-level heartbeat is needed.

### §7.5 — What this RFC does NOT add

- **Adaptive bitrate.** Out of scope. SDK consumers handle
  this in their codec layer.
- **Forward error correction.** Out of scope. The FEC layer
  belongs above the FFI in the codec/codec-wrapper.
- **Per-stream priority.** The kernel treats every stream
  with equal priority; HTTP/2 stream priority is left at gRPC
  defaults. A future amendment could expose per-stream
  weights, but no use case demands it now.

---

## §8 — Open questions (need user decision before P0)

The following points have a default answer chosen above but
warrant explicit confirmation:

| # | Question | Default | Rationale for default |
|---|---|---|---|
| Q1 | Add `key_frame`, `duration`, `dts` to BinaryChunk in v1? | YES | Forward-compatible (default zero/false; v1 PTY ignores). Without them, video is structurally broken. |
| Q2 | Add `StreamReady` control in v1? | YES | Multi-stream sync is impossible without it; cheap to add. |
| Q3 | Replay window size? | per-session (every sequence ever seen rejected) | Memory cost is `O(N frames)` per session — bounded by session lifetime; the alternative (sliding window) is harder to reason about. |
| Q4 | C ABI distribution: bundled with axon binary or separate `libeasynet_bidi.so`? | bundled | Same binary, same versioning. SDKs link via dlopen. |
| Q5 | Stall threshold? | 30s | Long enough for cellular RTT recovery, short enough that wedged clients don't accumulate. |
| Q6 | `mark_failed` reason size limit? | 1 KiB | Kernel allocates the slot eagerly; bound the worst case. |

---

## §9 — Implementation phases (post-approval)

**P0 — Conformance lock**
- CI grep: forbid `InvokeBidi` callers from setting
  `sequence` / `mac` directly outside `bidi_handler.rs` and
  the `bidi` shared module.
- Document the v1 sequence-replay rule and add a negative test
  per §5.1.

**P1 — Multimodal proto additions**
- Add `key_frame`, `duration`, `dts` to BinaryChunk.
- Add `StreamReady` to BidiControl.
- Regenerate Rust + Go + (later) Node/Python/Swift bindings.
- Backwards-compat tests: existing PTY round-trip continues
  to pass with no code change at the consumer.

**P2 — C ABI scaffolding**
- New crate `core/runtime-rs/src/ffi/bidi.rs` exposing the
  6 functions in §6.2.
- `cbindgen` generates `easynet_bidi.h`.
- Build artifact: `libeasynet_bidi.{so,dylib,dll}` shipped
  alongside the axon binary.

**P3 — Go SDK consumer**
- `sdk/go/easynet/bidi.go` wraps the C ABI via cgo.
- One end-to-end test: open a PTY session via the Go SDK,
  send `ls\n`, observe output bytes through the FFI.

**P4 — Backpressure + stall detection**
- Wire HTTP/2 stall detection (§7.4).
- Add `AXON_BIDI_RECEIVER_STALL` reason to the existing
  reason-code constants.

**P5 — Negative tests**
- Sequence replay rejection (§5.1).
- MAC chain break detection.
- Cross-direction key isolation.
- Slow-receiver stall + close.

Each phase ships independently; P0/P1/P2 are required before
any new ability adopts InvokeBidi (only PTY uses it today).

---

## §10 — Non-goals

This RFC explicitly does NOT:

1. Modify `Invoke` or `InvokeStream` semantics.
2. Touch the `SessionProvider` trait or `SessionRegistry` (RFC-002
   Stage 1 is final for this RFC's purposes).
3. Introduce new aggregate.* abilities (read-only catalog
   surface is final).
4. Expose `BidiStreamHandle` in `easynet-axon` SDK (the C ABI is
   the only public bidi surface for cross-language SDKs).
5. Implement Stage 2 / Stage 3 of RFC-002 — those remain blocked
   on the binary-topology decision.

---

## §11 — Approval gate

Please reply with one of:
- **"approved"** — proceed to P0.
- **"approved with changes"** — list the changes (revised RFC
  ships as v1.1).
- **"specify Q1..Q6"** — answer any of the open questions
  differently from the defaults; I revise and re-submit.

After approval, P0 implementation begins. No code is written
between this RFC and the approval reply.
