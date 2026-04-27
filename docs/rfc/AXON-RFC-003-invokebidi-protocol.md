# AXON-RFC-003 — InvokeBidi Data-Plane Protocol Specification

**Version**: **v1 (binding)** — frozen 2026-04-27 under
Silan.Hu sign-off. Three closure conditions verified:
execution closure (INV-1..10 §1.5), termination closure
(TERMINAL state §3.4 / §3.5), implementation closure
(FFI-INV-1..9 §5.8 + BP-INV-1..6 §6.6). Conformance is binary
and wire-indistinguishability is the cross-SDK acceptance test
(§9).
**Status**: binding protocol specification. No further semantic
changes without a v1.x amendment.
**Date**: 2026-04-27
**Author**: Claude (under Silan.Hu architectural authority)
**Scope**: task C-M1b — define the InvokeBidi data plane on top of
the now-correct chain-state model (G-bundle landed in
EasyNet-Axon `dff7294`).

**Change protocol**: this document is frozen for semantic content.
Errata (typos, broken cross-references, clarifying examples that
do not change a MUST / MUST NOT / SHOULD) may land directly. Any
change to §1–§6 normative text or to §1.5 / §3.4 / §3.5 / §5.8 /
§6.6 invariants requires a v1.x amendment that is itself signed
off by the authority before implementation may follow. Spec leads,
code follows (§9.5).
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

### §1.5 Frame validity invariants (formal, normative)

The role contract in §1.1 is the per-frame admissibility table.
The following invariants extend it across the whole frame
sequence; they are the **closure** of the per-frame rules under
execution. A receiver MUST treat any sequence violating them as
a protocol error and emit the terminal receipt with the relevant
`AXON_BIDI_*` reason from §1.4.

Notation: `up[i]` is the i-th frame on the up direction
(0-indexed); `down[i]` likewise. `chain(d, i)` is the chain state
on direction `d` after consuming `d[0..=i]`.

**INV-1 (anchor establishment)**
`up[0]` MUST be the FIRST frame consumed in either direction. No
`down[i]` is emitted before `up[0]` is admitted. `down[0]` is
the admission `InvocationReceipt` produced as a direct
consequence of `up[0]`.

**INV-2 (sequence monotonicity, per direction)**
For all `i ≥ 0` on direction `d`: `d[i].sequence == i`. Equivalently,
the i-th accepted frame on each direction has sequence exactly
i. The two directions are independent counters; cross-direction
interleaving on the wire is allowed.

**INV-3 (chain pre-condition)**
For all `i ≥ 1` on direction `d`: `d[i]` MAY be processed only if
`chain(d, i-1)` exists (i.e. `d[0..i]` were all admitted in
order). Frame i carries `mac` computed against the anchor
embedded in `chain(d, i-1)`. A receiver that has not yet
established `chain(d, i-1)` MUST NOT compute `chain(d, i)`.

**INV-4 (anchor parity)**
The 64-byte value placed in `up[0].mac` MUST equal the 64-byte
`envelope.caller_signature.signature` byte-for-byte. The HMAC
chain anchors on the value admission verified, never on a value
admission did not see. (Already enforced by §1.4
`AXON_BIDI_FRAME_ZERO_SIG_MISMATCH`.)

**INV-5 (uniqueness of EnvelopeOpen)**
Across the full session lifetime, exactly ONE frame in either
direction carries `EnvelopeOpen`: `up[0]`. Any subsequent
`up[i]` with `EnvelopeOpen` payload (including a re-sent `up[0]`
on a reconnect attempt within the same RPC) is a protocol
violation. The kernel emits `AXON_BIDI_DUPLICATE_OPEN`.

**INV-6 (terminal closure)**
A session has at most ONE terminal receipt frame. The terminal
frame is the LAST frame the kernel emits on the down direction.
After the terminal receipt:
- the kernel MUST NOT emit any further `down[i]`;
- the kernel MUST NOT process any further `up[i]` (per §3.5
  terminal-invalid rejection rule);
- both chain states are removed from runtime registries
  (`bidi_state_remove`, per §4.4).

A receiver that observes a frame after the terminal receipt MUST
discard it without processing.

**INV-7 (single anchor per session)**
The HMAC keys `(up_key, down_key)` are derived ONCE from
`up[0].mac` and `envelope.invocation_nonce`. They are never
rotated mid-session. Any v1 implementation that re-derives keys
mid-session is non-conformant.

**INV-8 (no half-admission)**
If `up[0]` admission fails (signature, membership, delegation,
or any §3.2 check), no chain state is created and no `down[0]`
is emitted. The gRPC stream is closed with a tonic `Status`
error. A caller observing tonic `Status::permission_denied` /
`invalid_argument` at open time MUST treat the stream as never
having existed.

**INV-9 (closure under timeout)**
A recv-side timeout (no down frame within the configured
window) does NOT mutate `chain(down, *)` and does NOT emit a
terminal receipt. The session remains alive; the caller MAY
retry recv. Timeout is the SOLE non-mutating Ok-shaped exit
from a recv call.

**INV-10 (causality: no down before signed up)**
For every `down[i]` with `i ≥ 1` carrying a `BinaryChunk` that
the ability handler produced from caller bytes: the caller bytes
were carried in some `up[j]` with `j < i`'s admission already
complete. The kernel guarantees this by serializing: `up[0]`
admission → ability dispatch → ability outputs flow as
`down[1..]`. Out-of-order admission is impossible because
admission is a single synchronous gate at frame 0.

These ten invariants close the frame schema. A receiver that
enforces §1.1 + §1.4 + §1.5 is conformant; a receiver missing
any single invariant is not.

### §1.6 What this spec deliberately does NOT add

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

### §3.4 Terminal state (cryptographic, not just runtime)

A session enters TERMINAL state at the FIRST occurrence of any
of:

(a) Kernel emits the terminal `InvocationReceipt` on the down
    direction (state ∈ {Completed, Failed, Cancelled, TimedOut}).
(b) Kernel observes a chain violation on the up direction
    (any `AXON_BIDI_FRAME_*` reason from §1.4).
(c) Caller observes a chain violation on the down direction
    (`AXON_BIDI_DOWN_*` from §1.4).
(d) Transport-level disconnect (gRPC `Status` carrying
    Disconnected, or TCP RST surfaced as an mpsc Disconnected).
(e) `up[i]` carries `BidiControl.eof = true` and is admitted.
(f) Caller invokes the bridge close-FFI.

TERMINAL is **monotonic and absorbing**: once a session is in
TERMINAL, no transition out exists.

### §3.5 Terminal-invalid rejection (closure of TERMINAL state)

After a session enters TERMINAL:

1. The kernel MUST NOT process any further `up[i]`. Any frame
   arriving on the up gRPC stream after TERMINAL is **silently
   discarded** (no MAC compute, no chain-state read, no
   `down[i]` produced in response). The gRPC up stream is
   drained by tonic and the transport socket closes.
2. The kernel MUST NOT emit any further `down[i]`. The down
   sender is dropped; tonic closes the down half of the gRPC
   stream.
3. Chain state for the session MUST be removed from the
   per-process registries (`BIDI_CHAIN_STATES` on the bridge,
   `SessionRegistry` lookup table on the kernel where
   applicable). After removal, any later FFI call referencing
   the same `stream_handle` MUST return `HANDLE_INVALID`
   (§5.2) with the bridge-side message
   `"stream_handle X is not a signed InvokeBidi stream"`.
4. The HMAC keys derived from `up[0].mac || nonce` MUST be
   considered **revoked** for cryptographic purposes. A
   conformant SDK MUST NOT reuse `up_key` / `down_key` for any
   subsequent operation. (Today's bridge does this implicitly
   by dropping the `BidiChainState` struct, which moves the
   key bytes into a heap allocation that gets dropped — there
   is no API path to extract the keys outside the chain
   state.)

This makes TERMINAL a **cryptographic** terminal, not just a
runtime convention. A frame tagged with the dead session's
`up_key` is invalid not because the kernel chooses to reject
it, but because the kernel has no representation of the chain
under which it could be valid.

**Why "silently discarded" not "rejected with code"**: a
tampered or replayed frame arriving after TERMINAL has no chain
state to anchor against; producing a fresh terminal receipt
would require a fresh chain anchor, which by INV-7 cannot
exist. The protocol's only conformant response is to drop. SDKs
that observe `HANDLE_INVALID` on the next FFI call know the
session is dead; they MUST NOT poll to extract a "what happened"
detail post-TERMINAL — that detail rode the terminal receipt
itself.

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

### §5.8 FFI behavioral invariants (formal, normative)

The §5.1 logical surface is the call shape. The following
invariants extend it to behavioral requirements every conformant
SDK MUST satisfy. They are the **closure of FFI semantics under
execution** — without them, "all SDKs implement the same FFI" is
true at the type level but false at the wire level.

**FFI-INV-1 (frame ordering preservation, recv side)**
`bidi_recv` MUST surface frames in the exact order the bridge's
chain-verify accepted them. SDKs MUST NOT reorder, batch, or
coalesce frames across `bidi_recv` calls. The k-th `bidi_recv`
that returns a non-`Timeout` `kind` returns the k-th
chain-verified down frame (counted from `down[0]`, the admission
receipt).

**FFI-INV-2 (frame ordering preservation, send side)**
`bidi_send` MUST commit frames to the up direction in the
order the FFI calls were issued. The bridge serializes
sequence allocation under a per-stream mutex; SDKs MUST NOT
reorder or merge sends. Two concurrent `bidi_send` calls on
the same stream produce two distinct frames whose sequence
ordering reflects mutex acquisition order — the SDK's caller
sees a definite "this send happened before that send" outcome.

**FFI-INV-3 (synchronous protocol-rejection surfacing)**
A protocol-level rejection (signature failure, MAC verify fail,
sequence skip, unknown stream id, …) MUST be surfaced
**synchronously** with respect to the FFI call that triggered
it. Specifically:
- `bidi_open`: protocol failure surfaces as `ENVELOPE_INVALID`
  status from the same call (no later "out of band" event).
- `bidi_send`: chain-state-not-found (post-TERMINAL retry)
  surfaces as `HANDLE_INVALID` from the same call. Transport-
  level send failure surfaces as `INTERNAL` or
  `CHANNEL_CLOSED` from the same call.
- `bidi_recv`: chain-violation surfaces as a return-value
  error (status code) on the recv that observed the violation,
  NOT later. Per §3.5, after the violation, subsequent recvs
  return `HANDLE_INVALID`.

SDKs MUST NOT defer protocol errors to a later FFI call. SDKs
MUST NOT silently swallow them and continue.

**FFI-INV-4 (no implicit retry)**
The FFI does NOT retry on the caller's behalf. A `bidi_send`
that fails with `INTERNAL` does NOT secretly resend; a
`bidi_recv` that times out does NOT re-poll. The caller decides
whether to retry. This invariant exists because the chain state
mutates on every commit; an implicit retry would mutate state
twice for one logical call, breaking sequence monotonicity
(INV-2).

**FFI-INV-5 (blocking is the default)**
v1 `bidi_send` and `bidi_recv` are blocking. A SDK that wraps
them in non-blocking constructs (Go goroutines, Node async,
Python asyncio) MUST do so by spawning a worker that calls the
blocking FFI; it MUST NOT introduce intermediate buffering
that decouples the caller's call site from the bridge's
acknowledgement. Specifically: the SDK's "send returned" event
MUST mean "the bridge committed the frame to the chain", not
"the SDK queued the send for later". Similarly for recv.

**FFI-INV-6 (status-code mapping is fixed)**
The status codes in §5.2 are the cross-SDK contract. Every
SDK MUST map them to language-idiomatic errors using a
DOCUMENTED 1:1 mapping. SDKs MUST NOT collapse multiple
status codes into one (e.g. mapping both `HANDLE_INVALID` and
`CHANNEL_CLOSED` to a single "stream gone" exception): the
caller's recovery action differs (`HANDLE_INVALID` ⇒ open new
session immediately; `CHANNEL_CLOSED` ⇒ session is in normal
TERMINAL state, drain receipt then open new). SDKs MAY add
language-specific subclasses but the discrimination MUST be
preserved.

**FFI-INV-7 (idempotent close)**
`bidi_close` MUST be idempotent. Calling it twice — or after
a `done` recv has already removed chain state — MUST return
without error. The second-call return value MAY differ
(`eof_sent: false` if the first call already emitted the eof)
but MUST be a successful return.

**FFI-INV-8 (no cross-stream effects)**
An FFI call against `stream_handle = X` MUST NOT mutate any
state belonging to `stream_handle = Y, Y ≠ X`. The per-stream
chain state isolation in the bridge enforces this; SDKs MUST
NOT introduce cross-stream side effects in their wrapper layer
(e.g. a shared retry queue that orders sends across streams).

**FFI-INV-9 (post-TERMINAL FFI behavior)**
After the session enters TERMINAL (§3.4), the next FFI call
of any kind on that `stream_handle` MUST return
`HANDLE_INVALID`. The SDK MUST NOT block waiting for a
response that will never come. The bridge enforces this via
`bidi_state_lookup` returning `BridgeBadRequest` immediately
once chain state is removed.

These nine invariants close the FFI contract. A new SDK that
satisfies the §5.1 surface but violates any of FFI-INV-1..9 is
non-conformant: cross-SDK determinism is broken.

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

### §6.6 Behavior under back-pressure (formal, normative)

The §6.4 guarantees describe the steady state. This section
fixes the behavior of every layer **when its capacity is
exhausted**, so that "v1 MUST hold rather than drop" has a
single concrete meaning across implementations.

**BP-INV-1 (Layer 1 HTTP/2 window full)**
When the HTTP/2 stream window is exhausted, the sending side
MUST block (the gRPC implementation handles this). It MUST
NOT drop frames, MUST NOT reorder, and MUST NOT silently
buffer past the window into a userland queue that bypasses the
window's authority. tonic and grpc-go both honor this by
default; v1 SDKs MUST NOT reconfigure HTTP/2 with `window_size
= ∞` or equivalent disable.

**BP-INV-2 (Layer 2 bridge mpsc full)**
When `request_buffer_size` or `chunk_buffer_size` is exhausted,
the side that produces (FFI for request, gRPC reader task for
chunk) MUST block its calling thread/task. It MUST NOT drop
frames and MUST NOT return a "would block" status to the
caller (v1 has no non-blocking variant; FFI-INV-5).

**BP-INV-3 (Layer 3 kernel handle mpsc full)**
When `BIDI_HANDLE_CHANNEL_DEPTH` (32) is exhausted in either
direction, the producer (`SessionProvider::send_chunk(...).await`
or `bidi_handler` forwarding an up-frame to the provider) MUST
await capacity. It MUST NOT drop frames and MUST NOT bypass
the bounded channel.

**BP-INV-4 (no cross-layer skip)**
A bound exhausted at layer N MUST propagate back to layer N-1
within bounded latency (the time for the calling thread to
observe the block + wake the upstream). v1 SDKs MUST NOT
introduce a side-channel between layers that lets data skip
past a full layer (e.g. an unbounded retry queue at the SDK
layer that drains the bridge's bounded mpsc into an unbounded
SDK buffer).

**BP-INV-5 (TERMINAL precedence over back-pressure)**
If a session enters TERMINAL (§3.4) while a producer is
blocked on a full layer, the block MUST be released within
bounded time (transport closure → mpsc Disconnected →
producer's `send.await` returns Err). The producer's `bidi_send`
FFI call MUST then return `CHANNEL_CLOSED` (or
`HANDLE_INVALID` if chain state has already been removed). It
MUST NOT block forever waiting for capacity that will never
appear.

**BP-INV-6 (no implicit drop)**
At no point in the v1 wire protocol does the kernel, the
bridge, or any SDK have permission to drop a BinaryChunk or
BidiControl frame to relieve back-pressure. Every frame
admitted at the producer's chain commit point MUST eventually
arrive at the consumer's chain verify point, OR the session
MUST enter TERMINAL with the relevant `AXON_BIDI_*` reason.
There is no third option.

This forecloses the "lossy v1" failure mode: any
implementation that silently drops to keep flowing under load
is non-conformant. Lossy semantics arrive in v2 via
`StreamDescriptor.ordering = "LOSS_TOLERANT"`, at which point
specific drop rules become legal under that ordering only.

### §6.7 Producer-side flow control responsibility

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

## §9 — Acceptance gate (semantic, not advisory)

### §9.1 Conformance is binary

A v1 InvokeBidi implementation is **either conformant or
non-conformant**. There is no partial conformance, no "mostly
conformant", no graduated levels.

A conformant implementation:

- **MUST accept** any frame sequence satisfying ALL constraints
  in §1.1, §1.4, and §1.5 (INV-1..INV-10), and produce the
  corresponding wire-visible behavior described in those
  sections.
- **MUST reject** any frame sequence violating ANY constraint,
  with the specific `AXON_BIDI_*` reason code from §1.4. The
  reason code is part of the contract, not a debugging hint.
- **MUST NOT accept** any frame sequence that violates a
  constraint, even if "the result would be reasonable." There is
  no implementation discretion to soften a MUST.
- **MUST NOT reject** any frame sequence that satisfies the
  constraints, even if "the result would be costly." There is
  no implementation discretion to harden a MUST NOT.

### §9.2 Acceptance is a semantic judgement

The acceptance test is NOT "does this implementation produce
the right output for these inputs?" — that is a unit-test
question. The acceptance test is:

> For every frame sequence S, does this implementation produce
> the response defined by §1–§6 + the conformance checklist of
> §8?

A conformant implementation MUST produce identical observable
behavior to every other conformant implementation, given
identical inputs. This is the cross-SDK determinism guarantee:
two conformant Go and Python SDKs MUST be wire-indistinguishable
to a recording observer.

### §9.3 Non-conformance is a protocol error

An implementation that satisfies the §5.1 logical surface but
violates any FFI-INV-1..9 (§5.8), or violates any BP-INV-1..6
(§6.6), or fails any conformance checklist item (§8), is
**non-conformant**. Non-conformance is not a quality issue, not
a bug to file, not a deferred-improvement item: it is a
**protocol violation**. The implementation is not a v1 InvokeBidi
implementation; it is something else that resembles one.

The correct remediation for non-conformance is to fix the
implementation, not to relax the spec. If §1–§6 demand something
infeasible for a given language or runtime, that is grounds for
a v1.x amendment to this spec, NOT grounds for shipping a
non-conformant SDK.

### §9.4 Sign-off conditions for v1

This spec is binding v1 when:

1. The shipped Rust implementations (`bidi_handler.rs`,
   `invoke_signed_bidi.rs`, `bidi.rs`, `session_provider.rs`)
   satisfy every MUST in §1–§6 + every invariant in §1.5 / §3.5
   / §5.8 / §6.6 verbatim. **Status: verified during round-1 +
   round-2 audits + G-bundle.**
2. The G-bundle (`dff7294`) is in place — chain-state lifetime
   matches §4.4. **Status: verified, 148/148 dendrite-bridge
   tests pass, 263/263 axon-runtime tests pass.**
3. The deferral list (§1.6 / §4.5 / §6.5 / §7) is the accepted
   v1 scope.

### §9.5 What "v1" means going forward

Once v1 is signed off:

- Any code change touching §1–§6 semantics requires a v1.x
  amendment to this document FIRST, then implementation. The
  spec leads, code follows.
- Any new SDK ships with the §8 conformance checklist filled
  in (every item ✓ or rationale for omission with sign-off).
- The cross-language hex anchors (HKDF, frame_mac) in
  `bidi.rs` are part of the contract. Drift triggers a wire
  break, not a "soft incompatibility."
- The forward-compat reservations (§2.3 `args_root_hash` field
  number, future `LOSS_TOLERANT` ordering, future `key_frame`
  / `duration` / `dts` BinaryChunk fields, future `StreamReady`
  control) are claimed; v1 producers MUST NOT use them, and v1
  consumers MUST tolerate them as protobuf-default-ignore on
  the wire.

### §9.6 Revision protocol (post-freeze)

This document is **frozen as v1** as of 2026-04-27. Two change
classes are recognised:

1. **Errata** — typos, broken cross-references, clarifying
   examples that do not change a MUST / MUST NOT / SHOULD.
   These land directly without amendment ceremony.

2. **v1.x amendments** — any change touching §1–§6 normative
   text or any of the formal invariant blocks (§1.5 INV-1..10,
   §3.4 TERMINAL state, §3.5 closure rule, §5.8 FFI-INV-1..9,
   §6.6 BP-INV-1..6). These require:
     - written rationale (single MUST or MUST NOT being
       changed, with the protocol property the change preserves
       or relaxes);
     - sign-off by Silan.Hu before implementation;
     - amendment lands as a separate document
       `AXON-RFC-003-amendment-vN.md` referencing the section
       and invariant being changed.

The "spec leads, code follows" rule (§9.5) is binding from this
freeze. Implementation patches that try to introduce a v1
behavior change without an amendment are protocol violations
and MUST be rejected at code review.

Future v2 work (lossy ordering, stall detection, cross-language
SDKs beyond Go, raw-byte FFI fast path, multimodal BinaryChunk
fields, StreamReady control) lives in
`AXON-RFC-004-invokebidi-v2.md` (not yet written) — v2 is a
NEW document, not an in-place rewrite of this one. v1 wire
remains binding for the lifetime of every v1 deployment.

---

End of spec.
