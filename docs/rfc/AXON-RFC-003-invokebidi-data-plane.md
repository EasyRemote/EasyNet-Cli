# AXON-RFC-003 — InvokeBidi Data-Plane: Status Audit + Gap List

**Status**: status audit, not new design.
**Date**: 2026-04-27
**Author**: Claude (under Silan.Hu architectural authority)
**Scope**: task C-M1b. The first draft of this RFC was written
as a greenfield design and was wrong — the protocol is largely
already implemented. This revision audits what exists and
enumerates the actual gaps.

---

## §0 — What's already shipped (the truth)

InvokeBidi is **not** a "to be designed" surface. The following
all exist in the current tree and are wired end-to-end:

| Layer | Status | File / location |
|---|---|---|
| Proto: `rpc InvokeBidi(stream up) returns (stream down)` | shipped | `EasyNet-Axon/core/proto/axon/v1/invoke.proto:100` |
| Proto: `EnvelopeOpen` / `BinaryChunk` / `BidiControl` / `StreamDescriptor` | shipped | same file, lines 419–544 |
| Proto: `PtyResize` / `PtySignal` / `MediaTimestamp` controls | shipped | same file, lines 528–544 |
| Axon kernel: `bidi_handler.rs` (frame 0 Ed25519, HKDF up/down keys, HMAC chain, replay, all `REASON_BIDI_*` codes) | shipped | `EasyNet-Axon/core/runtime-rs/src/services/invocation/bidi_handler.rs` |
| Axon kernel: `SessionProvider` + `BidiStreamHandle` (RFC-002 Stage 1) | shipped | `EasyNet-Axon/core/runtime-rs/src/services/invocation/session_provider.rs` |
| Axon kernel: dispatch via `SessionRegistry::attach_session_with_synthesis` | shipped | `EasyNet-Axon/core/runtime-rs/src/services/invocation/session_registry.rs` |
| CLI: `LocalAbilityRegistry::register_bidi` | shipped | `EasyNet-Cli/src/services/control/server.rs` (uses `bidi.echo` + `fleet.pty_session_attach`) |
| CLI: `fleet.pty_session_attach` BIDI handler | shipped | `EasyNet-Cli/src/runtime/agents/pty_attach_ability.rs` |
| Backend: `RealClient.InvokeBidi` (cgo path, ~200 LOC) | shipped | `EasyNet/backend/internal/axon/real_invoke_bidi.go` |
| Backend: `Fake.InvokeBidi` for tests | shipped | `EasyNet/backend/internal/axontest/fake.go` |
| Go SDK FFI: `dendrite_bridge_signed_invoke_bidi_cgo.go` + `_stub.go` | shipped | `EasyNet-Axon/sdk/go/easynet/` |

**Implication**: a frame-by-frame signed bidi session works
today, in production, end-to-end (Backend Go → cgo dendrite
bridge → Rust dylib → Axon kernel → CLI ability handler). PTY
attach exercises the full path on every interactive terminal
session through the web UI.

The first RFC-003 draft framed this as "to design" — that was
a mistake. The user pushed back ("等一下这个协议实际上我有
实现吧" — wait, this protocol is actually implemented, right?).
This revision corrects the framing.

---

## §1 — What C-M1b actually means in light of the audit

Task #130 reads "C-M1b (Axon): bidi multimodal InvokeStream —
proto + kernel + FFI". Re-reading against the shipped state:

- **"InvokeStream"** in the task title is a misnomer; the work
  is on **InvokeBidi**. (`InvokeStream` is server-stream-only
  and complete as of C-M1a.)
- **"proto + kernel + FFI"** are largely done. The
  *multimodal* qualifier is what's incomplete.
- The actual remaining work is narrow: multimodal-specific
  proto fields, codec-aware semantics, and SDK ergonomics for
  Go/Node/Python that the cgo path covers but doesn't
  document for cross-language consumers.

So C-M1b reduces to a **gap list**, not a design.

---

## §2 — Gap list (the actual to-do)

Each gap is independently shippable. Order is recommended but
not enforced; each one is small enough to land as one commit.

### Gap A — Replay-window failure code

**Status**: missing. `bidi_handler.rs` rejects out-of-order
frames (`AXON_BIDI_FRAME_SEQUENCE`) but has no distinct code
for the duplicate-sequence case (a frame N arrives whose
sequence equals a sequence already seen).

**Today**: a duplicate frame either:
- has matching MAC (impossible without prior secret) → would
  pass the chain check but be redundant;
- has wrong MAC → caught as `AXON_BIDI_FRAME_MAC_INVALID`.

**Why fix**: per-session replay rejection makes the security
property explicit and pins it in a negative test. Today the
guarantee is implicit (chain math protects it but no test
asserts it).

**Cost**: ~30 LOC + one negative test. New const
`REASON_BIDI_FRAME_REPLAY`.

### Gap B — Multimodal `BinaryChunk` fields

**Status**: PTY works because PTY needs nothing beyond
`(stream_id, data, pts)`. Video / multi-track audio need:
- `key_frame: bool` — receivers joining mid-stream need to
  know which frames are independently decodable.
- `duration: uint32` — Opus packets are 20ms, video varies;
  jitter buffers need to know.
- `dts: uint64` — codecs with B-frames have DTS != PTS.

**Today**: no production audio/video user. Adding the fields
is forward-compat (default zero/false; existing PTY consumers
ignore).

**Why fix**: without these fields, the first audio/video
ability that lands has to introduce them anyway. Adding now
costs nothing and removes a future blocker.

**Cost**: 3 proto field additions, regenerate Rust + Go
bindings, no semantic change in `bidi_handler.rs` (fields are
opaque to the kernel — they ride along with the frame and the
ability handler reads them).

### Gap C — `StreamReady` control

**Status**: missing. Multi-stream sessions (audio + video) need
the producer to signal "I'm about to start sending on stream X
with reference timestamp Y" so receivers can synchronise.

**Today**: irrelevant for PTY (single-stream, no PTS).

**Why fix**: same as Gap B — first multimodal ability needs it
or invents an ad-hoc workaround in `BidiControl.media_pts`.

**Cost**: 1 proto message + 1 oneof variant + zero kernel
changes (the kernel ignores `BidiControl` payload contents
beyond `eof`).

### Gap D — Slow-receiver stall detection

**Status**: not implemented. A receiver that opens an
InvokeBidi session and then stops reading frames holds the
kernel-side mpsc + a goroutine forever (until the gRPC
transport times out, which is hours by default).

**Today**: this hasn't surfaced because PTY sessions are short
and interactive (an idle PTY session times out via the shell,
not the wire).

**Why fix**: long-lived audio/video sessions amplify the
failure mode. A 1-hour zombie video session per user × 1000
users = 1000 wedged goroutines and full mpsc buffers.

**Cost**: HTTP/2 PING + ack-deadline check in `bidi_handler.rs`,
new `REASON_BIDI_RECEIVER_STALL` const, new tonic option for
the keepalive interval. ~80 LOC + one negative test.

### Gap E — Cross-language SDK FFI documentation

**Status**: the cgo path exists for Go (`dendrite_bridge_cgo.go`
+ `dendrite_bridge_signed_invoke_bidi_cgo.go`) and works in
production for Backend's `RealClient.InvokeBidi`. But:
- there's no documented C ABI consumable from Node/Python/Java/
  Swift;
- the cgo functions are Go-specific signatures (cgo `_Ctype_*`
  types), not portable C ABI;
- no `.h` header is generated/published.

**Today**: only Go consumes the FFI. Adding a non-Go SDK
requires re-deriving the C signatures from the Rust source.

**Why fix**: Node SDK (Frontend electron app), Python SDK
(notebook integration), and future Swift SDK (mobile) need
this. Today they'd have to use the gRPC client directly,
which means re-implementing the canonical encoder + HMAC chain
in each language — exactly the drift problem the FFI
prevents.

**Cost**: substantial. Three sub-tasks:
- E1: `cbindgen` to generate `easynet_bidi.h` from the existing
  Rust dylib exports (~20 LOC of Cargo metadata + a build
  script).
- E2: Node SDK wrapper (N-API, ~300 LOC).
- E3: Python SDK wrapper (ctypes, ~200 LOC).

E1 is the gating dependency for E2/E3. Each Node/Python wrapper
ships independently after E1.

### Gap F — `args_root_hash` decision (formal close)

**Status**: not needed. The first RFC-003 draft proposed
discussing this; on audit, every shipped consumer (PTY,
LLM-future, MCP-future) has args that fit comfortably in one
frame. PayloadStore (`transfer.proto`) covers the >4 MiB
outlier without inventing a Merkle mode.

**Why fix**: pin the decision in writing so a future PR doesn't
re-open the design discussion.

**Cost**: doc-only. One paragraph in `invoke.proto` near
`EnvelopeOpen.initial_args` saying "for args >4 MiB, use
PayloadStore upload + reference; do NOT chunk the envelope."

### Gap G — Backpressure semantics documented

**Status**: works correctly (HTTP/2 flow control + bounded
mpsc), but undocumented. Operators reasoning about stalled
sessions have to read the source.

**Why fix**: doc-only. Pins the existing behaviour so a future
"optimization" doesn't accidentally remove HTTP/2 flow control
in favour of something exotic.

**Cost**: a §A entry in the AXIOM checklist + a comment block
in `bidi_handler.rs`.

---

## §3 — What was wrong with the first RFC-003 draft

The prior draft proposed §1–§7 as if the protocol were greenfield.
Specifically:

| First-draft claim | Actual state |
|---|---|
| §1 "decision: keep InvokeBidi as distinct RPC" | Already distinct since P5-rewrite-15. Not a decision; a description of existing state. |
| §2 "decision: keep the existing schema" | Same — describing existing state. |
| §3 "decision: keep `args_digest`" | Already shipped; the question was "do we widen to root_hash" and the answer is no. |
| §4 "decision: keep existing single-Ed25519-anchor model" | Same — describing existing implementation. |
| §5 "keep existing chain shape" | Same — describing existing implementation. |
| §6 "FFI: 6-function C ABI" | Partially true — the Go cgo equivalent exists; portable C ABI doesn't. |
| §7 "rely on HTTP/2 flow control" | Same — describing existing behaviour. |

The first draft conflated "describe what exists" with "decide
the design". This revision separates them: §0 is the audit, §2
is the gap list.

---

## §4 — Recommended action

Three options; user picks:

**Option 1 — Land Gaps A + F + G (lowest cost, highest payoff)**
- Replay code (security pin).
- args_root_hash decision pinned (forecloses future design
  debate).
- Backpressure semantics documented (forecloses future "optimize"
  regression).

Estimate: 1 commit per repo, ~150 LOC + one negative test.
Unblocks no new abilities but pins three correctness invariants.

**Option 2 — Land Option 1 + Gap D (slow-receiver stall)**
- Adds the only correctness gap that affects production
  (long-lived sessions wedge goroutines).
- Test with a deliberately-stalled receiver.

Estimate: Option 1 + ~80 LOC.

**Option 3 — Land Options 1 & 2 + Gaps B + C (multimodal proto)**
- Adds the proto fields + control variant required for the
  first audio/video ability.
- No use case today, so no functional test possible — purely
  preparation.

Estimate: Options 1 & 2 + 4 proto field additions + Rust/Go
binding regen.

**Gap E (cross-language SDK FFI)** is intentionally separate.
It's a substantial multi-commit effort and not required by any
in-flight work. Recommend deferring until a Node/Python SDK
is actually being built.

---

## §5 — Approval gate

Pick:
- **Option 1 / 2 / 3** (one of the bundles above), OR
- specify a custom subset of Gaps A–G to land, OR
- defer C-M1b entirely (mark task #130 closed as "audit
  complete; gaps documented; no urgent functional work").

No code is written until you reply.
