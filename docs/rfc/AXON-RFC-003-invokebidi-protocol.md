# AXON-RFC-003 — InvokeBidi Data-Plane Protocol Specification

**Status**: protocol specification, no implementation required.
**Date**: 2026-04-27
**Author**: Claude (under Silan.Hu architectural authority)
**Scope**: task C-M1b — define the InvokeBidi data plane on top of
the now-correct chain-state model (G-bundle landed in
EasyNet-Axon `dff7294`).
**Companion documents**:
- `AXON-RFC-003-code-review.md` — the audit that produced this
- The shipped HMAC primitives in
  `core/runtime-rs/client-sdk/src/domain/bidi.rs`
- The shipped wire schema in
  `core/proto/axon/v1/invoke.proto`

This spec is **descriptive of what is shipped** plus **prescriptive
on the contracts every implementation MUST honor**. It does not
propose new code paths and it does not change existing ones.

---

## §0 — Conventions

- "MUST", "MUST NOT", "SHOULD", "MAY" follow RFC 2119.
- "frame 0" = the first frame in either direction (sequence == 0).
- "frame N" = a frame at sequence N ≥ 1.
- "session" = the lifetime of one open `InvokeBidi(stream up,
  stream down)` RPC, from envelope-signed open to terminal
  receipt.
- "chain anchor" = the value fed to `prev_mac` when computing the
  HMAC of the next frame. For frame 1 in either direction, the
  anchor is the 64-byte Ed25519 envelope signature; for frame N≥2
  it is the previous frame's 32-byte HMAC tag.
- All multi-byte integers are big-endian on the wire (proto
  varint elsewhere is irrelevant to the chain math; the chain
  math uses fixed-width big-endian sequence bytes).

---

## §1 — Frame schema

The wire is two protobuf streams, one in each direction:

```proto
service Axon {
  rpc InvokeBidi(stream InvokeBidiUp) returns (stream InvokeBidiDown);
}

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
    InvocationReceipt receipt      = 10;  // frame 0 + terminal
    BinaryChunk       binary_chunk = 11;
    BidiControl       control      = 12;
  }
}
```

### §1.1 Frame role contract

| Role | Up frame 0 | Up frame N≥1 | Down frame 0 | Down frame N≥1 |
|---|---|---|---|---|
| `EnvelopeOpen`     | REQUIRED | forbidden | forbidden | forbidden |
| `InvocationReceipt`| forbidden | forbidden | REQUIRED (admission accept) | optional (interim Running, REQUIRED for terminal) |
| `BinaryChunk`      | forbidden | allowed | forbidden | allowed |
| `BidiControl`      | forbidden | allowed | forbidden | allowed |

A receiver MUST reject any violation by closing the stream and
emitting the terminal receipt with an `AXON_BIDI_*` reason code
listed in §1.4.

### §1.2 BinaryChunk

```proto
message BinaryChunk {
  uint32 stream_id = 1;
  bytes  data      = 2;
  uint64 pts       = 3;
}
```

- `stream_id` references a `StreamDescriptor.stream_id` declared
  in `EnvelopeOpen.streams`. When `streams` is empty, only
  `stream_id == 0` is accepted (single-modal session). When
  `streams` has exactly one descriptor, `stream_id == 0` is also
  accepted as shorthand. With multiple descriptors, `stream_id`
  MUST exactly match a declared id.
- `data` is opaque to the kernel and to axon. The
  `StreamDescriptor.content_type` declares the codec / framing /
  container; the bridge is byte-blind here.
- `pts` is microseconds since the session reference clock
  established at `EnvelopeOpen`. Required for media (lip-sync);
  optional for PTY (which is intrinsically time-ordered by
  sequence). v1 receivers MUST tolerate `pts == 0` for any
  stream.

### §1.3 BidiControl

```proto
message BidiControl {
  oneof control {
    PtyResize       pty_resize  = 1;
    PtySignal       pty_signal  = 2;
    MediaTimestamp  media_pts   = 3;
    bool            eof         = 4;
  }
}
```

Variants are advisory to the ability handler. Axon does NOT
inspect them except for `eof`:

- `eof = true` on the up direction signals graceful close-up.
  The receiver MUST emit a terminal `InvocationReceipt` (state =
  `Completed`) as the final down frame and close the down sender.
- `eof = false` is reserved; v1 receivers MAY ignore.
- `pty_resize`, `pty_signal`, `media_pts` are forwarded verbatim
  to the ability handler (or, in the SessionProvider model, to
  the provider via `BidiStreamHandle::recv_control`). Unknown
  control variants in a future v2 are ignored by v1 receivers
  per protobuf default behaviour.

### §1.4 Receiver-rejection codes

The kernel emits these as the `reason` field of the terminal
receipt. SDKs grep for these strings; renaming any is a
protocol break.

| Code | When |
|---|---|
| `AXON_BIDI_FIRST_FRAME_NOT_OPEN` | up frame 0 payload is not `EnvelopeOpen` |
| `AXON_BIDI_FIRST_FRAME_SEQUENCE` | up frame 0 sequence != 0 |
| `AXON_BIDI_FRAME_ZERO_SIG_LEN` | up frame 0 mac length != 64 |
| `AXON_BIDI_FRAME_ZERO_SIG_MISMATCH` | up frame 0 mac != envelope.caller_signature.signature |
| `AXON_BIDI_OPEN_ENVELOPE_MISSING` | EnvelopeOpen.envelope == None |
| `AXON_BIDI_OPEN_TARGET_MISSING` | EnvelopeOpen.target == None |
| `AXON_BIDI_FRAME_MAC_LEN` | frame N≥1 mac length != 32 |
| `AXON_BIDI_FRAME_SEQUENCE` | frame N≥1 sequence != last_seen + 1 |
| `AXON_BIDI_FRAME_MAC_INVALID` | HMAC verify failed |
| `AXON_BIDI_DOWN_SEQUENCE` | (caller side) down frame sequence != last + 1 |
| `AXON_BIDI_DOWN_MAC_LEN` | (caller side) down frame mac length != 32 |
| `AXON_BIDI_DOWN_MAC_INVALID` | (caller side) down HMAC verify failed |
| `AXON_BIDI_UNKNOWN_STREAM_ID` | BinaryChunk.stream_id not in declared descriptors |
| `AXON_BIDI_NON_STRICT_ORDERING` | StreamDescriptor.ordering not "STRICT" or empty |
| `AXON_BIDI_DUPLICATE_OPEN` | second EnvelopeOpen frame mid-session |
| `AXON_BIDI_CALLER_DISCONNECT` | up gRPC stream closed without `eof = true` |

### §1.5 What this spec deliberately does NOT add

The following appear in the round-1 review's "gap list" but are
**explicitly out of scope for v1**:

- `BinaryChunk.key_frame` / `duration` / `dts` (multimodal
  enrichment; deferred until first audio/video ability ships)
- `BidiControl::StreamReady` (multi-stream synchronisation;
  deferred for the same reason)
- `LOSS_TOLERANT` ordering on `StreamDescriptor` (v2 amendment)
- Stall-detection on receiver inactivity (P1-3 in the audit)

v1 wire is final. Future additions land via additive proto
fields with default-zero/false semantics so v1 consumers continue
to parse forward-compatible v2 frames.

---

## §2 — args_digest (decision: keep)

`EnvelopeOpen.initial_args` carries the full canonical-encoded
args byte string inline in frame 0. The signed
`canonical_invocation_bytes` includes `args_digest = SHA-256(initial_args)`.

### §2.1 Wire contract

```proto
message EnvelopeOpen {
  Envelope envelope                 = 1;
  InvocationTarget target           = 2;
  bytes initial_args                = 3;   // full args, NOT a manifest
  string args_content_type          = 4;
  repeated StreamDescriptor streams = 5;
  map<string, string> metadata      = 6;
}
```

- Sender MUST place the FULL canonical-encoded args in
  `initial_args`.
- `args_digest` (used in `canonical_invocation_bytes` for the
  Ed25519 signature) is `SHA-256(initial_args)`.
- Receiver MUST recompute `args_digest` from the received
  `initial_args` bytes and verify against the signed envelope.
- Receiver MUST feed `initial_args` into the ability handler
  verbatim (no decoding by the bridge).

### §2.2 Why not args_root_hash

A Merkle-tree variant (`args_root_hash` over chunked args
streamed in subsequent BinaryChunk frames before ability
dispatch) would let envelopes commit to >gRPC-frame-size args
without inlining. v1 explicitly does not support this:

- Every shipped consumer (PTY, future LLM session, future MCP
  bridge) opens with small args (`{"session_id": "..."}`,
  ~30 bytes) and streams ability-specific data as BinaryChunks
  AFTER admission.
- gRPC's default 4 MiB max message size covers every realistic
  ability's open args.
- For >4 MiB args, the existing `transfer.proto` (PayloadStore)
  is the correct mechanism: upload the payload, place a payload
  reference in `initial_args`.

### §2.3 Forward compatibility

A future RFC needing genuinely streamed args MAY:
- add `EnvelopeOpen.args_root_hash bytes = 7` (next free tag);
- specify that when this field is non-empty, `initial_args` is
  interpreted as the manifest and subsequent BinaryChunks on a
  reserved `stream_id = 0` carry the args body;
- v1 receivers ignore unknown fields, so the addition is
  non-breaking.

The reserved field number 7 on `EnvelopeOpen` is documented here
as a forward-compat slot. v1 producers MUST NOT set it.

---

## §3 — Canonical signature anchor (frame 0)

Frame 0 in the up direction is the cryptographic root of the
session.

### §3.1 Construction (sender)

1. Build `Envelope` with all AXIOM seven-tuple fields populated
   except `caller_signature`.
2. Compute `args_digest = SHA-256(initial_args)` (see §2.1).
3. Build `canonical_invocation_bytes` per the existing
   `easynet_run_axon_client::admission::canonical_invocation_bytes`
   helper. This is the single canonical byte form shared with
   unary `Invoke` and server-stream `InvokeStream`.
4. Sign: `signature = Ed25519_sign(caller_priv, canonical_invocation_bytes)`.
5. Place the 64-byte signature in BOTH:
   - `envelope.caller_signature.signature`
   - `InvokeBidiUp.mac` (frame 0)
6. Build `EnvelopeOpen { envelope, target, initial_args,
   args_content_type, streams, metadata }` and place it in
   `InvokeBidiUp.payload.envelope_open`.
7. Set `InvokeBidiUp.sequence = 0`.

### §3.2 Verification (receiver)

The kernel runs admission EXACTLY ONCE per session, at frame 0.
Order of checks (already implemented in `bidi_handler.rs`):

1. `frame.sequence == 0` (else `AXON_BIDI_FIRST_FRAME_SEQUENCE`).
2. `frame.mac.len() == 64` (else `AXON_BIDI_FRAME_ZERO_SIG_LEN`).
3. `frame.payload` is `EnvelopeOpen` (else `AXON_BIDI_FIRST_FRAME_NOT_OPEN`).
4. `EnvelopeOpen.envelope` present (else `AXON_BIDI_OPEN_ENVELOPE_MISSING`).
5. `EnvelopeOpen.target` present (else `AXON_BIDI_OPEN_TARGET_MISSING`).
6. **Anchor parity check**:
   `frame.mac == envelope.caller_signature.signature` byte-for-byte
   (else `AXON_BIDI_FRAME_ZERO_SIG_MISMATCH`).
   This closes the downgrade vector where the chain anchors on
   bytes that admission did not see.
7. Stream descriptors: every `StreamDescriptor.ordering` MUST be
   `""` or `"STRICT"` (else `AXON_BIDI_NON_STRICT_ORDERING`).
8. Run the same admission gate quartet as unary `Invoke` and
   server-stream `InvokeStream`:
   - `signature_policy` (`enforce_signature_policy`)
   - `admission_gate::run_admission_gate` (signature verify
     against caller pubkey, nonce uniqueness, AXIOM tuple
     validity)
   - `membership_gate` (caller URI in directory or §A6 genesis
     exception)
   - `delegation_gate` (per §1.5 RFC-001 if subject ≠ caller)

If any gate fails the kernel returns a tonic `Status` (the gRPC
stream never opens) and no chain state is created.

### §3.3 Why a single Ed25519 anchor + HMAC chain (not per-frame Ed25519)

- Symmetry with unary `Invoke`: frame 0 is structurally identical
  to a signed unary envelope. One canonical encoder, one
  verifier, one admission machine.
- Steady-state cost: after admission the session needs only the
  32-byte derived HMAC keys. Re-signing every frame with Ed25519
  would be 100× slower at video frame rates.
- The HMAC chain (§4) provides integrity + ordering +
  anti-replay. Confidentiality is TLS's job.

---

## §4 — HMAC chain model (already implemented; reuse)

The chain math is FROZEN. v1 implementations MUST use the
shipped primitives in
`core/runtime-rs/client-sdk/src/domain/bidi.rs`:

```rust
pub const BIDI_HKDF_SALT: &[u8] = b"easynet-bidi-mac:v1";
pub const BIDI_INFO_UP: &[u8] = b"easynet-bidi-mac:v1/caller-to-callee";
pub const BIDI_INFO_DOWN: &[u8] = b"easynet-bidi-mac:v1/callee-to-caller";
pub const BIDI_MAC_LEN: usize = 32;        // truncated HMAC-SHA256
pub const BIDI_FRAME_ZERO_SIG_LEN: usize = 64;  // Ed25519 signature

pub fn derive_bidi_keys(envelope_signature: &[u8],
                        invocation_nonce: &[u8]) -> BidiKeys;
pub fn frame_mac(key: &[u8; 32],
                 sequence: u64,
                 prev_mac: &[u8],
                 canonical_payload: &[u8]) -> [u8; 32];
pub fn canonical_bidi_payload<M: BidiFrameWithMac>(frame: &M) -> Vec<u8>;
```

### §4.1 Key derivation

```
ikm  = envelope_signature || invocation_nonce
prk  = HKDF-SHA256-Extract(salt = BIDI_HKDF_SALT, ikm)

up_key   = HKDF-SHA256-Expand(prk, info = BIDI_INFO_UP,   L = 32)
down_key = HKDF-SHA256-Expand(prk, info = BIDI_INFO_DOWN, L = 32)
```

Both directions derive once at session open and reuse for the
session's lifetime. v1 SDKs MUST NOT rotate keys mid-session.

### §4.2 Per-frame MAC (frames N ≥ 1, both directions)

```
canonical_payload = prost_encode(frame_with_mac_field_zeroed)
tag = HMAC-SHA256(key,
                  sequence_be_8 || prev_mac || canonical_payload)
frame.mac = tag    // 32 bytes
```

- `key` is `up_key` or `down_key` per direction.
- `prev_mac` for frame 1 is the 64-byte Ed25519 signature; for
  frame N≥2 it is the previous frame's 32-byte tag. Up and down
  chains are independent counters but share the same anchor
  (envelope signature) at their respective frame 1.
- The `mac` field is cleared before encoding to avoid the
  chicken-and-egg of the field signing itself.
- Up sequence and down sequence are independent; cross-direction
  interleaving is allowed.

### §4.3 Cross-language anchor

The pinned hex outputs in `bidi.rs` lines 252–266 + 369–380 are
the cross-language contract. Any future port (Python, Node,
Swift) MUST reproduce identical bytes from identical inputs. The
test fixtures pinning these are mandatory parts of any new SDK.

### §4.4 Chain-state lifetime (post G-bundle)

This was the round-2 P0-A finding. After `dff7294`, chain state
in the bridge is removed on **every** exit path:

| Exit path | Trigger | Mechanism |
|---|---|---|
| Server EOF (`done = true`) | recv sees done | explicit `bidi_state_remove` |
| Explicit close-FFI | caller calls close | explicit `bidi_state_remove` |
| recv chain-violation | sequence/MAC/length error | wrapper-level cleanup |
| send transport error | bidi_stream_send_impl Err | explicit `bidi_state_remove` |
| TCP RST / process drop | transport observer fires | `STREAM_HANDLES` close hook → `on_transport_close` |
| Session-wide teardown | `close_streams_for_session` | observer fires for each victim handle |
| Recv timeout | timeout branch | **NOT removed** (chain still alive; caller may retry) |

The chain anchor is FRESH on every new session; once a session
ends (any path above except timeout), the chain is gone.

### §4.5 What this section does NOT add

- Replay-window negative test (P1-1, deferred).
- Per-direction AEAD layered on top of HMAC (v2 amendment).
- Liveness pings (HTTP/2 keepalive at the transport layer is
  the v1 mechanism; chain-level pings are out of scope).

---

## §5 — FFI raw-byte API (Send / Recv)

The cross-language data plane is a C ABI. SDKs in Go / Node /
Python / Swift consume it via cgo / N-API / ctypes / Swift C
interop. Today the bridge exports four signed-bidi verbs through
JSON-base64 FFI; this section specifies the **logical contract**
those verbs satisfy. A future raw-bytes-pointer fast-path FFI
(gap E in the audit) is a different transport for the same
contract.

### §5.1 Logical surface

```
// Open one signed bidi session. Runs admission inside.
//   Inputs:  envelope (proto-encoded EnvelopeOpen frame 0
//                      with mac filled in)
//   Returns: stream_handle (opaque u64) on success
bidi_open(envelope_bytes, envelope_len) -> { stream_handle, status }

// Send one up-frame. Bridge fills sequence, computes MAC,
// commits chain state under per-stream mutex.
//   payload variants: BinaryChunk, PtyResize, PtySignal,
//                     MediaTimestamp, Eof
bidi_send(stream_handle, payload) -> { sent_sequence, status }

// Block for one verified down-frame.
//   timeout_ms <= 0 means default (30s today).
//   Returns one of: BinaryChunk, Receipt, Control, Done, Timeout
bidi_recv(stream_handle, timeout_ms) -> { kind, sequence, ... }

// Best-effort graceful close: emits eof control, drops chain
// state, leaves transport for caller to drain.
bidi_close(stream_handle) -> { eof_sent, status }
```

### §5.2 Status codes

```
OK                = 0   // success
WOULD_BLOCK       = 1   // future non-blocking variant; v1 always blocks
CHANNEL_CLOSED    = 2   // session terminal observed; further calls fail
INVALID_ARG       = 3   // payload validation failed at FFI boundary
HANDLE_INVALID    = 4   // stream_handle unknown (use-after-close, or
                        //   the post-G-bundle "session is dead, open
                        //   a new one" signal — §4.4)
ENVELOPE_INVALID  = 5   // open-time only (signature verify failed,
                        //   admission rejected, etc.)
INTERNAL          = 6   // kernel/bridge bug; logged on the bridge side
```

SDKs map these to language-idiomatic exceptions / errors / Result.

### §5.3 What the FFI MUST NOT expose

- Sequence numbers (computed bridge-side).
- MAC bytes (computed bridge-side).
- HKDF keys (derived bridge-side, never leaves).
- Raw protobuf bytes (SDKs build envelope_bytes via per-language
  protoc-gen; that is the only proto contact).
- Chain state internals (bridge-private).

This is what keeps cross-language SDKs from drifting on the
chain math. The bridge is the single canonical encoder.

### §5.4 Concurrency contract

- Per-stream chain state is serialised by a `Mutex<BidiChainState>`
  on the bridge side. Two concurrent sends OR two concurrent
  recvs on the same stream are allowed but their ordering is
  determined by mutex acquisition (nondeterministic from the
  caller's view).
- Send and recv on the same stream from different threads run
  concurrently — they touch disjoint chain-state fields
  (`last_up_*` vs `last_down_*`). The mutex serialises briefly
  during canonical-encode + HMAC compute and releases before
  the underlying transport call.
- v1 SDKs SHOULD use one goroutine/task for sends and another
  for recvs (the natural producer/consumer split).

### §5.5 Lifecycle: what "session is dead" looks like to the SDK

After the G-bundle (§4.4), an SDK observes session death as:
- `bidi_recv` returns `kind = "done"` (server EOF or terminal
  receipt followed by transport close), OR
- any subsequent `bidi_send` / `bidi_recv` returns
  `HANDLE_INVALID` with the bridge-side message
  `"stream_handle X is not a signed InvokeBidi stream (no chain state)"`.

This is the explicit "open a new session" signal. SDKs MUST
treat `HANDLE_INVALID` after a chain-violation Err as terminal
(do not retry on the same handle).

### §5.6 Transport boundary today

Today's bridge FFI is JSON-base64 in / JSON-base64 out, defined
by `axon_dendrite_invoke_bidi_{open,send,recv,close}_signed_json`.
For a 1 MiB BinaryChunk:
- Encode: bytes → base64 (~1.33×) → JSON wrap
- Decode (other end of FFI): JSON → base64 → bytes

This works today and is correct. It is also slow at media
bitrates (gap E). A future raw-bytes-pointer FFI variant is a
**transport optimization**, not a protocol change. The chain
math, frame schema, lifecycle semantics, and cross-language
contracts above are independent of how bytes cross the FFI.

### §5.7 String memory ownership (FFI hygiene)

Every `*mut c_char` returned by the bridge MUST be released via
`axon_dendrite_string_free`. Forgetting this leaks memory
permanently (round-2 audit P3-9). A future `cbindgen`-generated
header will document this contract in the generated `.h`.

---

## §6 — Backpressure semantics

The data plane has three back-pressure layers. All three MUST
function for end-to-end flow control to work; removing any one
turns the others into best-effort observation.

### §6.1 Layer 1: HTTP/2 stream flow control

- **Authority**: the gRPC transport. tonic (server) and
  grpc-go (client) both implement this correctly out of the box.
- **Mechanism**: HTTP/2 receivers advertise a window (default
  64 KiB); senders stall when window exhausted.
- **Behaviour**: a slow recv-side cannot be overrun by a fast
  send-side. The TCP socket fills, the kernel back-pressures
  the userland write, and the producer goroutine/task
  transparently waits.

### §6.2 Layer 2: bridge bounded mpsc

- **Authority**: the dendrite bridge. `request_buffer_size` and
  `chunk_buffer_size` (defaults set by `BidiOptions`) bound the
  per-direction mpsc that buffers between the FFI thunk and the
  tokio gRPC writer/reader task.
- **Mechanism**: `tokio::mpsc::channel(N)` with bounded N.
  Producer (FFI side) blocks on full; consumer (tokio side)
  blocks on empty.
- **Behaviour**: absorbs bursts smaller than N past Layer 1.
  When Layer 1 also fills, Layer 2 fills, and the producer's
  `bidi_send` FFI call blocks.

### §6.3 Layer 3: kernel-side bounded mpsc (provider handle)

- **Authority**: the axon kernel. `BIDI_HANDLE_CHANNEL_DEPTH = 32`
  (`session_provider.rs:327`).
- **Mechanism**: per-direction `tokio::mpsc::channel(32)` between
  bidi_handler's frame loop and the SessionProvider's pump.
- **Behaviour**: provider that produces faster than the wire
  drains blocks on `BidiStreamWriter::send_chunk(...).await`.

### §6.4 What v1 MUST guarantee

- A producer that ignores back-pressure (busy-loops calling
  `bidi_send` ignoring blocks) MUST NOT crash the bridge or
  the kernel. Every layer MUST hold (block) rather than drop.
- A receiver that stops reading MUST NOT cause the kernel
  to allocate unbounded buffers. Layer 1's window + Layer 3's
  bounded mpsc + Layer 2's bounded mpsc together cap the
  worst-case in-flight data per session.

### §6.5 What v1 explicitly does NOT do

- **Lossy frames**. Every BinaryChunk reaches the receiver in
  order or the chain breaks. There is no "drop on full" path.
  Audio/video deployments that need lossy semantics will get
  `LOSS_TOLERANT` ordering in v2.
- **Slow-receiver timeout**. P1-3 from the audit. A receiver
  who opens a session and then stops reading holds:
  - one tokio task in `run_frame_loop`
  - 256-slot Layer 1 mpsc
  - 32-slot Layer 3 mpsc per direction
  - the underlying PTY / LLM / MCP backend
  …until the gRPC transport-level timeout fires (hours). This
  is a **known gap**, deliberately deferred per the user's
  current scope. Production deployments mitigate by:
  - configuring HTTP/2 keepalive aggressively at the tonic
    server settings layer
  - bounding session count per caller at the application layer
- **Per-stream priority**. All BinaryChunks within a session
  ride the same gRPC stream; HTTP/2 stream priority defaults
  apply. Multi-stream priority weighting is out of scope.
- **Adaptive bitrate / FEC**. SDK consumers handle these in
  their codec layer, above the FFI.

### §6.6 Producer-side flow control responsibility

When `bidi_send` blocks past a deadline the producer cares
about, the SDK SHOULD:
1. Cancel the send attempt with the language's idiomatic
   timeout (Go `context.WithTimeout`, etc.).
2. Decide application-level: retry with smaller chunk, drop
   the chunk and continue (codec-aware), or close the session
   (`bidi_close`).

The kernel does NOT make codec-specific drop decisions. SDK
consumers do, on top of the v1 "always-blocking" baseline.

---

## §7 — Out of scope (explicitly)

This spec does not address:

1. **Stall detection** (P1-3). Deferred. v1 relies on HTTP/2
   keepalive + caller responsibility.
2. **Replay negative tests** (P1-1). Deferred. Replay rejection
   is structural in the chain math; explicit negative test pin
   is a future hardening commit.
3. **Multimodal BinaryChunk extensions** (gap B from audit:
   `key_frame`, `duration`, `dts`). Defer until first audio /
   video ability is actually being built.
4. **`StreamReady` control variant** (gap C). Same deferral.
5. **Raw-bytes FFI variant** (gap E). Independent transport
   optimization; the protocol contract is unchanged.
6. **Chain-state logic changes**. The G-bundle (`dff7294`) is
   the final lifetime model for v1. Any further changes to chain
   state require a new RFC.

---

## §8 — Conformance checklist

Any new SDK or wire-compatible re-implementation MUST satisfy:

- [ ] Reproduces the cross-language HKDF anchor at `bidi.rs:252–266`.
- [ ] Reproduces the cross-language `frame_mac` anchor at
      `bidi.rs:369–380`.
- [ ] Multi-frame chain wedge property (one byte flipped in
      frame N changes every tag from N onward).
- [ ] Frame 0 anchor parity check (`frame.mac ==
      envelope.caller_signature.signature`).
- [ ] All `AXON_BIDI_*` rejection codes from §1.4 produced
      under the documented conditions.
- [ ] BinaryChunk `stream_id` validation per §1.2 rules.
- [ ] `eof = true` on up direction produces a Completed
      terminal receipt.
- [ ] All chain-state exit paths from §4.4 remove the chain
      state (or for the timeout case, explicitly leave it).
- [ ] FFI string ownership: every returned `*mut c_char` is
      freed via `axon_dendrite_string_free` (or the cbindgen
      equivalent).
- [ ] Producer flow control: no busy-loop crashes the bridge
      or the kernel; every layer blocks rather than drops.

---

## §9 — Acceptance gate

This spec is ready for sign-off if:

1. The shipped code in `bidi_handler.rs`, `invoke_signed_bidi.rs`,
   `bidi.rs`, and `session_provider.rs` matches every "MUST"
   in §1–§6 verbatim. (Verified during round-1 + round-2 audits.)
2. The G-bundle (`dff7294`) is in place — chain-state lifetime
   matches §4.4. (Verified: 148/148 dendrite-bridge tests pass,
   263/263 axon-runtime tests pass.)
3. The §1.5 / §4.5 / §6.5 / §7 deferral list is acceptable as
   the v1 scope.

If accepted: this becomes the binding C-M1b protocol document.
Subsequent C-M1b execution items (cbindgen header, multimodal
proto fields, stall detection, replay test) reference this doc
and amend section-by-section rather than re-deriving.

If changes wanted: respond with section numbers to revise; this
file becomes v0.1, the revision lands as v1.

---

End of spec.
