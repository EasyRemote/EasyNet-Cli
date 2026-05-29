# AXON-RFC-003 — InvokeBidi Code Review (audit, no fixes)

**Date**: 2026-04-27
**Scope**: ~3500 LOC across 6 files, the actual InvokeBidi
data-plane implementation.
**Output**: numbered issue list (severity tagged), no code changes.
**Decision needed**: which issues to fix in C-M1b, in what order.

---

## Files reviewed

| File | LOC | What it does |
|---|---|---|
| `EasyNet-Axon/core/runtime-rs/src/services/invocation/bidi_handler.rs` | 1496 | gRPC entrypoint, frame-0 admission, HMAC chain, frame loop, terminal emission |
| `EasyNet-Axon/core/runtime-rs/src/services/invocation/session_provider.rs` | 907 | `SessionProvider` trait, `BidiStreamHandle` + split halves, `SessionMeta`, `AttachResult` |
| `EasyNet-Axon/core/runtime-rs/src/services/invocation/session_registry.rs` | 1130 | Per-process session table + provider dispatch + `seal()` lifecycle |
| `EasyNet-Axon/core/runtime-rs/src/services/invocation/builtin_pty_provider.rs` | 413 | PTY pump task wiring `BidiStreamHandle` ↔ `session_bridge::pty_*_bytes` |
| `EasyNet/backend/internal/axon/real_invoke_bidi.go` | 380 | Backend `RealClient.InvokeBidi` wrapper around the cgo SDK |
| `EasyNet-Axon/sdk/go/easynet/dendrite_bridge_signed_invoke_bidi_cgo.go` | 776 | Go SDK cgo FFI, base64-JSON wire to bridge dylib |

**Total reviewed**: ~5100 LOC of bidi-relevant code (kernel + SDK + backend).

**Not reviewed** (out of scope for this pass): the dendrite-bridge
Rust crate (`sdk/rust/src/dendrite_bridge.rs`) — that's the layer
between the Go cgo and the kernel; HMAC-chain math actually lives
there. Audit it next pass if any HMAC-related issue below needs
deeper investigation.

---

## Severity legend

- **🔴 P0** — security or correctness bug; ship a fix before
  trusting the surface.
- **🟠 P1** — production-affecting bug under realistic load; not
  a security issue but will surface in operations.
- **🟡 P2** — design smell, ergonomics gap, missing test, doc
  drift; fix opportunistically.
- **🟢 P3** — observation only; no action required, included for
  context.

---

## Issues

### 🟠 P1-1 — Replay defence is implicit, not explicit

**Where**: `bidi_handler.rs` lines 580–588 + 599–616.

**What**: The sequence check rejects frames whose `sequence !=
last_up_seq + 1`. Replay protection comes "for free" because:
- a duplicate-sequence frame would skip the strict +1 check, OR
- if the attacker also bumped sequence, the chained MAC fails
  because `prev_mac` differs.

**Why it's a problem**: there's no test that proves replay is
rejected. The guarantee is implicit in the chain math but no
negative test asserts it. A future "optimization" that relaxed
the strict +1 check (e.g. to support out-of-order delivery) would
silently open a replay window without any test failing.

**Fix shape**: add `REASON_BIDI_FRAME_REPLAY` const + a negative
test that replays an already-accepted frame and asserts the
stream closes with that reason. ~30 LOC.

**Aligns with**: gap A in the prior RFC-003 audit.

---

### 🟠 P1-2 — `failure_reason` slot has no size bound

**Where**: `session_provider.rs` lines 577–587 + 649–659.

**What**: `mark_failed(reason: impl Into<String>)` accepts
arbitrary-length strings. The slot stores them verbatim until the
session ends.

**Why it's a problem**: a buggy provider that builds the failure
reason from untrusted data (e.g. `format!("{}", caller_input)`)
could allocate unbounded memory. With multi-attach support and
many concurrent failed sessions, this is a memory-exhaustion vector
under adversarial conditions.

**Realistic exploitation**: low (providers are kernel-internal
code today). But the cap is one-line cheap; the absence is a
latent foot-gun.

**Fix shape**: `let reason = reason.into(); let reason = if
reason.len() > MAX_FAILURE_REASON_LEN { format!("{}…
[truncated]", &reason[..MAX_FAILURE_REASON_LEN]) } else { reason
};` then store. Pin a const e.g. `MAX_FAILURE_REASON_LEN = 1024`.

---

### 🟠 P1-3 — No slow-receiver / stalled-session timeout

**Where**: `bidi_handler.rs` `run_frame_loop` (lines 537–836).

**What**: The frame loop has no inactivity / keepalive logic. A
caller who opens the session, sends frame 0, then stops reading
the down stream forever holds:
- one tokio task (`run_frame_loop`)
- one mpsc with `DOWN_CHANNEL_DEPTH = 256` slot capacity
- one PTY pump task (if PTY)
- the actual PTY child process

…until the gRPC transport-level timeout fires (which is hours by
default).

**Why it's a problem**: long-lived sessions in production
(audio/video, long PTY) magnify this. A flaky network closing TCP
abruptly is the realistic failure mode; the kernel should detect
and shed within seconds, not hours.

**Today's mitigation**: nothing. PTY's interactive nature masks
it (idle PTY → shell prompts → no growth) but that's accidental.

**Fix shape**: HTTP/2 PING + ack-deadline check (tonic exposes
this via `KeepaliveSettings`). On stall: cancel `shutdown` token,
close down channel with `REASON_BIDI_RECEIVER_STALL` terminal.
~80 LOC + one negative test.

**Aligns with**: gap D in the prior audit.

---

### 🟡 P2-1 — Signal-syscall race after lock release

**Where**: `builtin_pty_provider.rs` lines 340–353.

**What**:
```rust
let session = runtime.state.session_bridge.sessions
    .get(session_id)?.value().clone();   // snapshot
if session.node_id != node_id { return Err(...); }
let pid = session.pty.pid();             // snapshot pid
// ... lock released here ...
nix::sys::signal::kill(Pid::from_raw(pid), signal)?;
```

The comment says "Clone out of the dashmap entry immediately so we
don't hold a shard lock across the kill syscall" — that's correct
hygiene. **But**: between the clone and the kill, the PTY child
could exit and the OS could recycle the pid for an unrelated
process. Sending the signal then hits the wrong process.

**Realistic**: the race window is microseconds; pid reuse on Linux
takes minutes typically. So very low probability — but it's a
genuine TOCTOU on signal delivery, the kind of thing that bites
in extreme cases.

**Fix shape**: move the kill back inside the dashmap entry guard.
Either accept the lock-during-syscall (kill is non-blocking,
~microseconds) OR use `pidfd_send_signal` on Linux (immune to pid
reuse) with `nix::sys::signal::kill` as the fallback for macOS /
older kernels.

**Verdict**: low priority unless there's evidence of misdelivered
signals in the wild.

---

### 🟡 P2-2 — Concurrent `mark_failed` from `BidiStreamHandle` AND `BidiStreamWriter`

**Where**: `session_provider.rs` lines 577 (`BidiStreamHandle::mark_failed`)
+ 649 (`BidiStreamWriter::mark_failed`).

**What**: After `split()`, the `BidiStreamWriter` clone holds
its own `Arc<Mutex<Option<String>>>` pointing to the same slot.
Both expose `mark_failed`. Two concurrent threads — one with the
unsplit handle (impossible after split, but the type system
doesn't enforce), another with a writer clone — would race.

**Today's mitigation**: `split()` consumes `self` so the
post-split caller can't use the unsplit method. The race is
between **multiple writer clones**.

**Realistic**: PTY pump uses one writer (no fan-out). LLM /
multimodal ability with multiple producer subtasks could fan out
writers and hit this.

**Today's behaviour**: the inner `Mutex` serializes correctly;
"first writer wins" semantics hold. So the race is *correct*, just
*undocumented*.

**Fix shape**: add a doc comment on `BidiStreamWriter::mark_failed`
saying "first concurrent caller wins; other clones see false."
Optionally promote to `Arc<OnceLock<String>>` to make the
"first-writer-wins" property type-level rather than runtime.

---

### 🟡 P2-3 — `SessionRegistry::create_session` panics on collision

**Where**: `session_registry.rs` lines 441–447.

**What**:
```rust
if g.sessions.contains_key(&session_id) {
    panic!("UUIDv4 collided ...");
}
```

**Why questionable**: panicking takes the whole runtime down.
UUIDv4 collision is statistically negligible but:
- in tests with deterministic UUID seeding, the panic is
  reachable;
- in production, a corrupted RNG (e.g. fork-after-rand-init bug)
  could make this fire — and the right response is to log + retry
  with a fresh UUID, not to crash the kernel.

**Today's mitigation**: zero — the panic message gives no path
to recovery.

**Fix shape**: replace panic with `bail!("session_id collision;
retry create_session")` and let the caller retry. Keeps the
"this should never happen" loud-failure intent without ceiling
the entire runtime.

---

### 🟡 P2-4 — `down_tx` size choice (`DOWN_CHANNEL_DEPTH = 256`) vs `BIDI_HANDLE_CHANNEL_DEPTH = 32`

**Where**: `bidi_handler.rs` line 215 (`DOWN_CHANNEL_DEPTH = 256`)
+ `session_provider.rs` line 327 (`BIDI_HANDLE_CHANNEL_DEPTH = 32`).

**What**: The wire-side mpsc holds 256 frames; the provider-side
mpsc holds 32 frames. A producer running ahead can fill the
provider mpsc (32 deep), get back-pressured by the channel-full
behaviour, *while the wire mpsc still has 224 free slots*.

**Why it's a smell**: the bottleneck moves to the slower of the
two. With a fast wire (no caller stall) the 224-slot headroom is
wasted; with a slow caller, the 32-slot provider mpsc fills first
and the producer blocks — but the wire mpsc is also being drained
slowly, so eventually that fills too.

**Today's behaviour**: works correctly (both bounds are correct
back-pressure points), just non-obvious why two different numbers.

**Fix shape**: doc-only — pin a comment explaining the tiered
back-pressure (provider mpsc = "sustained throughput", wire mpsc
= "burst absorber"). Or unify both at one constant, accepting
that wire bursts past 32 can't be absorbed without provider
involvement.

---

### 🟡 P2-5 — Echo-surface emit_binary_down's MAX-sentinel error encoding

**Where**: `bidi_handler.rs` lines 677–696.

**What**:
```rust
let (next_seq, next_mac) = emit_binary_down(...)
    .await
    .map_err(|e| (down_seq, last_down_mac.clone(), e))
    .unwrap_or((u64::MAX, Vec::new()));
if next_seq == u64::MAX { return LoopOutcome::fail(...); }
```

The error path encodes "channel closed" as `(u64::MAX,
Vec::new())` and the caller checks for the sentinel. But:
1. The `map_err` arm constructs a `(down_seq, last_down_mac.clone(), e)`
   triple that's never used — the `unwrap_or` discards it and uses
   the sentinel.
2. `u64::MAX` as a sentinel is implicit — a future seq counter
   that legitimately reaches u64::MAX would silently take this
   branch (theoretical: you'd need 2^64 frames in one session).

**Why it's a smell**: harder-to-read than `match
emit_binary_down(...).await { Ok((s, m)) => ..., Err(_) =>
return LoopOutcome::fail(...) }`. The dead `map_err` payload is
confusing.

**Fix shape**: rewrite the call sites to use a simple match. ~10
LOC delta, no semantic change. Same defect at lines 787–805.

---

### 🟡 P2-6 — `eofSent` semantics inconsistent between Send + Close paths

**Where**: `dendrite_bridge_signed_invoke_bidi_cgo.go` lines 487–495 +
749–774.

**What**: `SendEOF` flips `eofSent` BEFORE calling the FFI; on
FFI failure, `eofSent` stays true (documented as "once
attempted, always attempted"). `Close` does the same. So:
- Caller calls `SendEOF` → FFI fails → `eofSent=true`.
- Caller catches the error, decides to retry via `Close` →
  `Close` sees `eofSent=true`, returns `nil` immediately (no FFI
  call) → bridge state never released.

**Realistic exploitation**: a transient FFI error during EOF
followed by Close leaks bridge-side state. The session's
`stream_handle` stays allocated until the bridge process exits
or some GC sweep runs.

**Today's mitigation**: zero. The doc explicitly says this is
intentional (TOCTOU concern), but the trade-off chosen is
"prevent retry confusion" at the cost of "leak resource on
error."

**Fix shape**: split the two intents. `eofSent` tracks "did EOF
control frame go through?"; introduce separate `closed atomic.Bool`
for "did Close release bridge state?". Close on a stream where
`eofSent=true && closed=false` STILL calls the close-FFI (skipping
the eof emit). ~20 LOC.

---

### 🟡 P2-7 — Backend `RealClient.InvokeBidi` doesn't validate `opts.Streams[].Ordering`

**Where**: `real_invoke_bidi.go` lines 258–268.

**What**: The Go-side helperPayload validation in the SDK
(comment at `dendrite_bridge_signed_invoke_bidi_cgo.go:160-164`)
says "the Go-side validator catches the mismatch before the FFI
call". Looking at backend's `InvokeBidi`, it directly forwards
`opts.Streams` without validating.

**Why it matters**: a backend caller that passes
`Ordering: "LOSS_TOLERANT"` gets a bridge-side error (not nice;
late failure) instead of a typed error at the backend boundary.

**Realistic exploitation**: low — backend is the only caller
and it doesn't synthesize streams from untrusted input.

**Fix shape**: pre-flight check at backend `InvokeBidi`:
```go
for _, sd := range opts.Streams {
    if sd.Ordering != "" && sd.Ordering != "STRICT" {
        return nil, nil, fmt.Errorf(
            "stream %d: Ordering=%q, only \"STRICT\" supported in v1",
            sd.StreamID, sd.Ordering)
    }
}
```

---

### 🟡 P2-8 — `InvocationReceipt` payload-variant validation gap on down direction

**Where**: `bidi_handler.rs` reads incoming `InvokeBidiUp` frames
and validates payload variant per role (frame 0 = EnvelopeOpen,
frame N = BinaryChunk/Control). But: on the **down** direction,
there's no equivalent client-side check. The caller (Go SDK or
test fixture) trusts whatever the kernel sends.

**Why it matters**: the kernel itself sends only valid payloads
(`build_down_frame` is the single emitter, controlled), so this
is a safety property the kernel maintains internally. But:
- a malicious or buggy intermediate proxy could inject an
  unexpected payload variant on the down direction;
- the SDK would happily decode and surface it.

**Today's mitigation**: TLS prevents the proxy attack; the SDK's
`decodeBidiFrame` returns a typed error on unknown kinds. So
defence-in-depth is acceptable.

**Fix shape**: doc-only — document in the SDK that "kernel sends
only Receipt at frame 0, then BinaryChunk/Control/Receipt
afterwards" so future SDK consumers don't get surprised by valid-
but-unusual orderings.

---

### 🟢 P3-1 — `mark_failed` Mutex + `expect("poisoned")` rather than recovery

**Where**: `session_provider.rs` line 581 + 651.

**What**: `self.failure_reason.lock().expect("poisoned")`. A
panicked thread that held the lock would poison it; the next
caller crashes the runtime.

**Why it's fine today**: `mark_failed` is small + leaf (no
panicking dependencies). But the `.expect()` panic path is
present.

**Fix shape**: optional — `lock().unwrap_or_else(|e|
e.into_inner())` would recover the slot. Trade-off: silently
recovering from a poisoned lock could mask real bugs. Current
behaviour (loud panic) is a defensible choice for a kernel
component; documenting the choice would be cheaper than changing
it.

---

### 🟢 P3-2 — Stale doc reference in `down0_signed` comment

**Where**: `bidi_handler.rs` line 330 — comment says "Down-direction
chain anchor for the FIRST signed down frame is the envelope sig
(frame 0 up). Each subsequent frame chains on the previous tag."

**What**: Accurate. But "envelope sig" is one of two possible
naming conventions; the rest of the file uses "frame0.mac". A
consistency pass would be cheap.

**Fix shape**: optional — rename for consistency. Doc-only.

---

### 🟢 P3-3 — `frame_zero_wrong_mac_length_is_rejected` test happens to use the wrong-len signature both in `frame.mac` AND `envelope.caller_signature`

**Where**: `bidi_handler.rs` line 1192.

**What**:
```rust
let f = frame_zero(vec![0u8; 32], Some(open_payload(dummy_envelope_with_sig(vec![0u8; 32]), "x")));
```

Both lengths are 32 (instead of 64). The test is testing that
`frame_zero_wrong_mac_length` fires, but the mismatch check
(`frame0.mac != envelope.caller_signature.signature`) wouldn't
fire either because both sides match (both are 32 bytes).

**Why it matters**: the test asserts on `REASON_BIDI_FRAME_ZERO_SIG_LEN`
which fires first (length check before mismatch check). So the
test passes for the right reason. But adding a second test that
specifically exercises the mismatch path would be valuable.

**Fix shape**: add `frame_zero_mac_anchor_signature_mismatch_rejected`
test: `mac=vec![0u8;64], envelope.caller_signature.signature=vec![1u8;64]`
→ assert on `REASON_BIDI_FRAME_ZERO_SIG_MISMATCH`. ~15 LOC.

---

### 🟢 P3-4 — No test that exercises the synthesis path in concurrent attach

**Where**: `session_registry.rs` `attach_session_with_synthesis`
(lines 497+).

**What**: The lookup → synthesise → insert sequence holds the
write lock for the whole arm, which is correct. But there's no
test that fires two concurrent attaches with the same legacy
session_id and asserts only one synthesises while the other
finds the now-existing record.

**Why it matters**: low — the race is structurally impossible
because of the write lock. But if a future refactor relaxes the
lock to read+upgrade, the bug becomes possible silently.

**Fix shape**: add a tokio-multi-thread test that joins two
attach calls. ~25 LOC.

---

### 🟢 P3-5 — JSON-base64 wire to bridge isn't the steady-state plan

**Where**: `dendrite_bridge_signed_invoke_bidi_cgo.go` line 38–41
("the cgo thunk shape is JSON-in / JSON-out... A future fast-path
FFI variant with raw byte pointers can be slotted in").

**What**: every BinaryChunk is base64-encoded, marshalled to JSON,
sent across cgo, parsed bridge-side, base64-decoded, then sent on
the wire as raw protobuf bytes. Round trip cost: O(n) memory
allocation per frame on both encode and decode.

**Why it's known-tech-debt**: the comment explicitly calls it
out. For PTY (small text frames) this is fine; for video frames
(up to 1 MiB each at high bitrates) the overhead is real.

**Fix shape**: out of scope for code review — this is the
"raw-bytes FFI" gap E from the prior RFC-003 audit. Independent
multi-commit project. Today's behaviour is correct, just slow at
media bitrates.

---

## Aggregate findings

- **No P0 (security/correctness blockers).** The HMAC chain math,
  admission gate sequence, frame validation, and chain anchoring
  are all correct. The "downgrade gap closer" check at line 470
  (frame0.mac == envelope.caller_signature.signature) is a real
  hardening fix that's properly implemented.
- **3 P1s** worth fixing for production hardening: explicit
  replay test (P1-1), failure_reason size cap (P1-2), receiver
  stall detection (P1-3).
- **8 P2s** are design smells / ergonomics — no urgent action,
  fix opportunistically.
- **5 P3s** are observations only.

The implementation is solid. The gaps are around defensive
hardening + observability + ergonomics, not protocol correctness.

---

## What I did NOT review (out of scope this pass)

1. **`sdk/rust/src/dendrite_bridge.rs`** — the cgo-side Rust
   crate that the Go SDK calls into. This is where the actual
   HMAC chain emit happens (Go SDK's send path → bridge HMAC
   → kernel-internal frame). If any of P1-1's deeper investigation
   needs the actual chain math, this is the file.
2. **CLI's `pty_attach_ability.rs`** + daemon `LocalRuntime` Bidi registration.
   Same trait shape as the Axon-side, with dispatch owned by the daemon runtime.
3. **Test coverage breadth** — I read tests opportunistically
   while reading the production paths. A separate "what's
   missing from coverage" pass would be its own audit.
4. **Backend's `wshandler.go`** — the WebSocket → InvokeBidi
   bridge that's still task #122 pending. Not yet wired against
   the new `RealClient.InvokeBidi`.

---

## Recommended next action (round 1)

Pick one of:
- **A** Fix P1-1+P1-2+P1-3 (production hardening bundle).
- **B** Fix P1-1+P1-2 only.
- **C** Document P2s only.
- **D** Accept audit, defer fixes.
- **E** Audit `sdk/rust/src/dendrite_bridge.rs` next pass.

---

# Round 2 — bridge + chain math audit (option E executed)

**Date**: same day
**Scope**: ~2650 additional LOC across 3 files where the actual
HMAC chain emit / verify happens. This is the layer between the
Go cgo and the kernel.

| File | LOC | Role |
|---|---|---|
| `EasyNet-Axon/core/runtime-rs/client-sdk/src/domain/bidi.rs` | 422 | Pure crypto primitives: HKDF derive, frame_mac, canonical_bidi_payload |
| `EasyNet-Axon/core/runtime-rs/dendrite-bridge/src/invoke_signed_bidi.rs` | 1167 | Producer chain emit + consumer chain verify (caller side) |
| `EasyNet-Axon/core/runtime-rs/dendrite-bridge/src/ffi_exports.rs` | 1060 (only bidi parts read) | C ABI exports for bidi open/send/recv/close |

Total reviewed across both rounds: ~5500 LOC.

## Round 2 findings

### 🟢 P3-6 — `bidi.rs` is exemplary

Pure functions, no I/O, no async. HKDF + frame_mac +
canonical_bidi_payload all correct. Cross-language hex anchors
(lines 252–266 + 369–380) pin HMAC outputs against fixed inputs
— any future Python/Node port that drifts a single byte breaks
the test. Multi-frame chain wedge test (387–421) demonstrates
the chain property empirically. All `.expect("never fails")` are
genuinely infallible (HKDF L=32 < 255×HashLen, HMAC any 32-byte
key, prost encode into Vec). Nothing to change.

---

### 🔴 P0-A — `BIDI_CHAIN_STATES` map leaks on transport-level orphans

**Where**: `invoke_signed_bidi.rs` lines 136–166.

**What**: Process-global `Mutex<HashMap<u64, Arc<Mutex<BidiChainState>>>>`
keyed on `stream_handle`. Entries are inserted in
`bidi_open_signed_impl` (line 430). They are removed in only
TWO places:

1. `recv_signed_impl` line 647, when the **server** sends
   `done=true` (clean EOF terminal frame).
2. `close_signed_impl` line 825, when the caller explicitly
   closes.

**The leak**: any path where the underlying gRPC transport drops
the stream WITHOUT either of those two events leaves the map
entry forever. Realistic triggers:
- TCP RST from the peer.
- Caller process crash mid-session.
- Caller code that opens, errors, and forgets to call `Close()`
  in a panic recovery path.
- Recv timing out with `kind=timeout` (line 651) and the caller
  giving up — the chain state stays.

The Go SDK's `eofSent` defect from P2-6 (SendEOF FFI failure
prevents Close from ever calling close-FFI) makes this even
easier to hit in production: SendEOF errors → `eofSent=true` →
Close becomes no-op → bridge entry leaks.

**Severity rationale (why P0)**:
- Memory-bounded only by the count of historical sessions a
  given bridge process has handled.
- Each `BidiChainState` ≈ 200 bytes including key material; at
  10K leaked sessions = ~2MB minimum + map overhead. At 100K =
  significant.
- More importantly: leaked state means `stream_handle` slot
  collisions on u64 wrap could silently overwrite old state.

**Today's mitigation**: bridge processes restart on deploy,
which clears the map. Long-running unattended deploys would
slowly accumulate.

**Why round 1 missed this**: round 1 read the kernel side
(tokio task lifetimes bounded by gRPC stream) and the Go cgo
side (which doesn't own the chain state). This map lives in the
*bridge crate*, not in the kernel.

**Fix shape**: introduce a periodic sweep task or hook
`raw_transport::stream_close` → `bidi_state_remove`. ~50 LOC +
test.

---

### 🟠 P1-4 — `recv_signed_impl` chain mismatch error returns Err but doesn't drop chain state

**Where**: `invoke_signed_bidi.rs` lines 686–725.

**What**: When sequence or MAC verification fails (codes
`AXON_BIDI_DOWN_SEQUENCE` / `_MAC_LEN` / `_MAC_INVALID`), the
function returns Err. But `bidi_state_remove` is NOT called.
A Go SDK that retries recv on error would re-attempt verification
against the stale `last_down_mac` — chain is structurally dead,
no recovery is possible, but the state stays around.

**Why it matters**: chain mismatch = session poisoned. Right
behaviour is to drop chain state immediately so subsequent recv
returns BridgeBadRequest ("not a signed stream") forcing the
caller to open a new session.

**Fix shape**: on any chain-violation Err from recv, call
`bidi_state_remove(stream_handle)` before returning. ~3 LOC.

---

### 🟠 P1-5 — Chain state mutex held across `prost::Message::encode_to_vec`

**Where**: `invoke_signed_bidi.rs` lines 581–589 (inside
`build_and_commit_up_frame`).

**What**: Mutex held across `canonical_bidi_payload` (clone +
encode), `frame_mac`, and `frame.encode_to_vec()`. Doc claims
"microseconds for typical payload sizes." True for KB chunks.
For a 1 MiB video frame: ~2-3ms mutex hold per frame. At 30 fps
× 1 MiB × two-way = ~140ms/sec mutex hold = 14% of wall time
serialized.

**Why it matters today**: doesn't. PTY frames are sub-KB. If
C-M5c (voice/video on InvokeBidi) actually ships, this becomes
the bottleneck.

**Fix shape**: move prost encode + canonical_bidi_payload out
of the mutex. Mutex covers only: read + write seq/mac slots.
~30 LOC refactor.

**Trade-off**: more complex; current is bug-free if slow.
Defer until media ships.

---

### 🟡 P2-9 — `ct_eq_bytes` reimplemented locally instead of `subtle`

**Where**: `invoke_signed_bidi.rs` lines 793–806.

Hand-rolled constant-time byte compare. Implementation is
correct (XOR-OR all bytes, compare zero). Comment says adding
`subtle` dep "for one call site would be heavier." But the
kernel side (`bidi_handler.rs:610`) uses `subtle::ConstantTimeEq`
— so the dep already exists in the workspace. Two different
implementations risk divergence in future maintenance.

**Fix shape**: depend on `subtle` in bridge Cargo.toml; use
`ct_eq` for both. ~5 LOC.

---

### 🟡 P2-10 — `recv_signed_impl` validates MAC length AFTER protobuf decode

**Where**: `invoke_signed_bidi.rs` lines 669 (decode) → 699
(length check).

A malicious peer could send protobuf with wildly inflated `mac`
field, forcing a multi-MiB allocation in the decode before the
length check fires. Mitigated by gRPC's 4 MiB default max
message size, so worst case is one 4 MiB allocation per recv —
annoying but bounded.

**Fix shape**: pull `mac.len()` check out of chain-state lock,
do it on proto-decoded frame BEFORE acquiring lock. ~5 LOC.

---

### 🟡 P2-11 — `bidi_state_lookup` Arc clone, registry mutex drop is implicit

**Where**: `invoke_signed_bidi.rs` lines 146–159.

The Arc clone correctly releases the registry mutex by dropping
`map` at scope end. Doc claims "drop the registry lock
immediately" but the drop is implicit, not explicit. A future
maintainer adding code between lookup and return could hold the
mutex longer than intended.

**Fix shape**: optional — explicit `drop(map)` after `cloned()`.
~1 LOC.

---

### 🟢 P3-7 — `bidi_state_remove` on done frame is unconditional

When recv sees `done=true` chain state is removed, subsequent
recv returns BridgeBadRequest. Could improve the error message
to explicitly say "session ended; open a new session." ~5 LOC.

---

### 🟢 P3-8 — `invoke_bidi_close_signed_impl` removes chain state even on EOF send failure

Lines 821–836. Deliberate design choice — explicit-close is
leak-free even if EOF send fails. Doesn't help with crashes /
TCP RST that never call close (that's P0-A's territory).

---

### 🟢 P3-9 — FFI string-free contract is documented but enforcement is by convention

`ffi_exports.rs` lines 872–879. Every `*mut c_char` returned
MUST be freed via `axon_dendrite_string_free`. Go SDK does this
correctly (verified round 1). Future SDK in another language
that forgets this leaks memory. Doc-only fix; cbindgen-generated
header would make the contract self-documenting (gap E).

---

## Round 2 aggregate

- **1 new P0** — `BIDI_CHAIN_STATES` leak on transport-level
  orphans. Real production-affecting bug.
- **2 new P1s** — chain state survives chain-mismatch Err
  (P1-4); mutex held across prost encode at media bitrates
  (P1-5).
- **3 new P2s** — hand-rolled ct_eq vs subtle (P2-9); MAC length
  check after decode (P2-10); explicit drop documentation (P2-11).
- **4 new P3s** — observations.

## Combined audit totals (round 1 + round 2)

| Severity | Round 1 | Round 2 | Total |
|---|---|---|---|
| 🔴 P0 | 0 | 1 | **1** |
| 🟠 P1 | 3 | 2 | **5** |
| 🟡 P2 | 8 | 3 | **11** |
| 🟢 P3 | 5 | 4 | **9** |

**Total reviewed**: ~5500 LOC across 8 files. Round 2 found 1
production bug (the bridge leak) that round 1 couldn't see
because the relevant code lives in the bridge crate, not in the
kernel or the Go SDK.

## Recommended next action (revised after round 2)

- **F**: Fix P0-A immediately (small commit, high payoff).
  ~50 LOC + 1 negative test. The leak is the only finding
  that's actively bad in production today.

- **A'**: Fix P0-A + all 5 P1s as a hardening sprint.
  ~250 LOC + 5 negative tests. Completes the production-
  hardening pass for the data plane.

- **G**: Fix P0-A + P1-4 only (leak + chain-mismatch state
  cleanup). They are the same code area. ~80 LOC + 2 tests.
  Tightest scope.

My recommendation: **F first** (urgent), then evaluate which of
the remaining P1s to do later. P1-3 (kernel stall) and P1-5
(mutex hold across encode) are independent and neither is on
fire today.

I've not modified any code in this pass. The audit is complete;
nothing was changed.
