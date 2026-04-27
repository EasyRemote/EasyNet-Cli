# AXON-RFC-003 v1 — Verification Note

**Purpose**: consolidate the v1 binding state into a single
auditable receipt. A future reviewer can answer "what is v1,
what's verified, what's deferred" from this file alone, without
reconstructing the audit trail from git history or the four
companion documents.

**Not a roadmap**, not a design document, not a v2 plan. Just
the receipt for what shipped, what's tested, and what's
intentionally absent.

**Frozen**: 2026-04-27, same date as the v1 binding sign-off.

---

## §1 — v1 binding spec + corresponding commits

The binding spec is `AXON-RFC-003-invokebidi-protocol.md` at
revision committed in `EasyNet-Cli` on the `rfc-001-impl`
branch.

### v1 spec lineage

| Stage | EasyNet-Cli commit | What landed |
|---|---|---|
| Design phase (C-M1b) | `939964a` | First spec draft (audit-style) |
| 4 surgical patches | `69b8ec7` | INV-1..10, TERMINAL semantics, FFI-INV-1..9, BP-INV-1..6, acceptance MUST |
| Freeze ceremony | `9df2684` | Version line set to "v1 (binding)"; §9.6 post-freeze revision protocol |

The v1 spec body is **frozen for semantic content**. Errata
land directly; v1.x amendments require a separate file
(`AXON-RFC-003-amendment-vN.md`) + sign-off + spec leads code
follows (§9.5). v2 work goes in a NEW document
(`AXON-RFC-004-invokebidi-v2.md`, not yet written).

### v1 implementation lineage

The kernel implementation that v1 verifies against lives in
`EasyNet-Axon` on the `rfc-001-impl` branch.

| Implementation milestone | EasyNet-Axon commit | What it gives v1 |
|---|---|---|
| RFC-002 Stage 1 (SessionProvider trait + registry + Kernel::seal) | `ab8a4e6` and 9 self-review passes through `c8130e7` | Chain-state lifecycle that v1 §3.4 / §3.5 build TERMINAL semantics on top of |
| C-M11 federation.subscribe_directory | `fe4295b` | Server-stream ability that exercises the InvokeStream sibling RPC; not v1-direct but proves the strict-separation directive (v1 §1) |
| G-bundle (P0-A leak + P1-4 stale anchor) | `dff7294` | The chain-state lifetime guarantees that §3.5 / §4.4 specify |
| X-batch1 P1-1 + P1-2 | `aa932ba`, `d938b04` | Replay negative tests + failure_reason size cap |
| X-batch2 P1-3 | `4a3fc9f` | Stall detection — TERMINAL precedence over back-pressure (v1 §6.6 BP-INV-5) |
| X-batch3 conformance suite | `1bd6bd0` | The 21-test conformance reference module |

### Acceptance gate (v1 §9.4) — status

The three sign-off conditions of the v1 spec map to:

1. ✅ **Shipped Rust matches every MUST in §1–§6 + invariants
   in §1.5 / §3.5 / §5.8 / §6.6 verbatim.** Verified via the
   round-1 + round-2 audits (`AXON-RFC-003-code-review.md`),
   the G-bundle landing, and the X-batch3 conformance suite.
2. ✅ **G-bundle (`dff7294`) is in place — chain-state lifetime
   matches §4.4.** Verified.
3. ✅ **Deferral list (§1.6 / §4.5 / §6.5 / §7) is the accepted
   v1 scope.** Reaffirmed in §3 of this note.

---

## §2 — X path test coverage map

The X path is the chain of work that turned v1 from
"论证 closure" into "CI-verified closure."

### Test deltas per batch

| Batch | EasyNet-Axon commit | axon-runtime | dendrite-bridge | What's pinned |
|---|---|---|---|---|
| baseline (pre-G) | — | 250 | 145 | shipped behavior |
| G-bundle | `dff7294` | 263 (+13) | 148 (+3) | chain-state lifetime per §3.5 / §4.4 |
| X-batch1 | `aa932ba` + `d938b04` | 272 (+9) | 148 | replay rejection (3 tests) + failure_reason cap (6 tests) |
| X-batch2 | `4a3fc9f` | 276 (+4) | 148 | stall detection (4 tests) |
| X-batch3 | `1bd6bd0` | **297 (+21)** | 148 | minimal six-category conformance reference |

**Total v1 verification delta**: +47 axon-runtime tests,
+3 dendrite-bridge tests across the X path. All green; cross-
language signature matrix tests (5/5) unchanged across the
entire X path.

### Coverage by RFC-003 v1 invariant block

This is the audit map for v1 §9.1 ("conformance is binary"):

| Invariant block | Where pinned | Pin shape |
|---|---|---|
| §1.1 frame role contract | bidi_handler tests + cat 1 (7 tests) | wire-string + behavioral |
| §1.4 wire rejection codes | cat 5 prefix test + per-code wire-string pins | wire-string |
| §1.5 INV-1 (anchor establishment) | cat 1 frame-0-first | wire-string + behavioral (validate_frame_zero per-module) |
| §1.5 INV-2 (sequence monotonicity) | cat 3 chain wedge + bidi_handler skip-ahead test | behavioral |
| §1.5 INV-3 (chain pre-condition) | bidi_handler chain test + cat 3 | behavioral |
| §1.5 INV-4 (anchor parity) | bidi_handler frame-0-mac-mismatch + cat 2 | behavioral + wire-string |
| §1.5 INV-5 (uniqueness of EnvelopeOpen) | cat 1 duplicate-open + bidi_handler test | wire-string |
| §1.5 INV-6 (terminal closure) | cat 4 structural pin (single emit site) | structural |
| §1.5 INV-7 (single anchor per session) | implicit in §3.5 closure rule (chain-state remove) | structural (no key rotation API) |
| §1.5 INV-8 (no half-admission) | bidi_handler four-gates-before-keys test | structural |
| §1.5 INV-9 (closure under timeout) | X-batch2 stall tests (timeout NOT removed) | behavioral |
| §1.5 INV-10 (causality) | bidi_handler frame-loop ordering | structural |
| §3.4 TERMINAL state | dendrite-bridge G-bundle tests + cat 4 | behavioral + structural |
| §3.5 closure rule (silent-discard, key revocation) | G-bundle wire-up + cat 4 emit-site count | structural |
| §4.x HMAC chain math | client-sdk/domain/bidi cross-language hex anchors | behavioral |
| §4.4 chain-state lifetime | G-bundle tests (every exit path covered) | behavioral |
| §5.2 status codes | cat 5 distinctness + prefix tests | wire-string |
| §5.8 FFI-INV-1..9 | per-SDK conformance (Go cgo verified; Node/Python/Swift TBD per SDK) | per-SDK behavioral |
| §6.6 BP-INV-1 (HTTP/2 window) | tonic transport defaults | implicit |
| §6.6 BP-INV-2 (bridge mpsc) | dendrite-bridge tests | implicit |
| §6.6 BP-INV-3 (kernel handle mpsc) | session_provider tests + cat 6 | structural |
| §6.6 BP-INV-4 (no cross-layer skip) | structural pin (no unbounded queues anywhere) | structural |
| §6.6 BP-INV-5 (TERMINAL precedence) | X-batch2 stall path | behavioral |
| §6.6 BP-INV-6 (no implicit drop) | cat 6 try_send absence + DOWN_CHANNEL_DEPTH | structural |

**What this map does NOT claim**:
- Each invariant has at least ONE pin somewhere. Most have
  multiple (behavioral + structural + wire-string).
- Pin coverage ≠ exhaustive property-based coverage. The
  X-batch3 brief explicitly chose "minimal reference" over
  "full matrix."
- Per-SDK FFI-INV coverage exists ONLY for the Go cgo SDK
  today. Future Node/Python/Swift ports MUST reproduce the
  conformance pins in their language; the kernel side is
  language-agnostic.

### Conformance reference

- **Single auditable file**: `core/runtime-rs/src/tests/rfc003_bidi_v1_conformance.rs`
- **Mode**: in-tree integration test (axon-runtime is a binary
  crate; tests/ external dir cannot reach internals)
- **Six categories**, each at least one test, plus one meta
  test asserting all six categories remain present
- **Three test shapes coexist deliberately**: wire-string
  pin, behavioral pin, structural grep pin — documented in
  the file header

---

## §3 — Explicit non-goals + deferred v2 items

This section is the **mirror image** of v1 §1.6 / §4.5 / §6.5 /
§7. Listing them again here means a future reviewer doesn't
have to cross-reference the spec to know "did v1 deliberately
not do X."

### Things v1 deliberately does NOT include

These are non-goals of v1, not bugs:

1. **Multimodal BinaryChunk fields** (`key_frame`, `duration`,
   `dts`) — needed by audio/video; intentionally absent in v1.
2. **`BidiControl::StreamReady`** — multi-stream synchronisation;
   needed when multimodal lands.
3. **`LOSS_TOLERANT` ordering** on `StreamDescriptor` — v1 is
   STRICT-only (lossy streams introduced in v2).
4. **`args_root_hash`** on `EnvelopeOpen` — v1 keeps
   `args_digest` (single SHA-256 over inline `initial_args`);
   field number 7 is reserved for the future Merkle variant.
5. **Per-direction AEAD layered on HMAC** — v1 leaves
   confidentiality to TLS; HMAC chain provides
   integrity + ordering + anti-replay only.
6. **Liveness pings at the chain layer** — v1 relies on
   HTTP/2 keepalive + the X-batch2 stall detection. No
   chain-level heartbeat frames exist.
7. **Per-stream priority** — all BinaryChunks within one
   session ride the same gRPC stream at default HTTP/2
   priority weights.
8. **Adaptive bitrate / FEC** — codec layer concerns above
   the FFI; not in v1 protocol scope.
9. **Raw-bytes-pointer FFI fast path** — v1 ships the
   JSON-base64 cgo path that's correct but slow at media
   bitrates. Future v2 work introduces the fast path
   without changing the protocol contract.

### Items deferred to a future RFC-004 v2

If multimodal / lossy / faster FFI work begins, it lives in
`AXON-RFC-004-invokebidi-v2.md` (not yet written). v2 is a
NEW document, not an in-place rewrite of v1. v1 wire remains
binding for the lifetime of every v1 deployment per §9.6.

The v2 design surface (when it begins) covers at minimum:

- The four BinaryChunk fields above
- `StreamReady` control variant
- `LOSS_TOLERANT` ordering with explicit drop rules
- `args_root_hash` Merkle variant on `EnvelopeOpen` field 7
- Optional AEAD layered on HMAC for compromised-TLS-terminator
  threat models
- Raw-bytes FFI variant (`cbindgen` header, libeasynet_bidi.so)
- Per-language SDK ports (Node, Python, Swift, Java) +
  cross-language conformance harness consuming the X-batch3
  pins as language-agnostic test fixtures

These are listed for forward-compat reference only. None of
them is in scope for v1; none of them has a planned start
date.

### Items deferred to a future RFC-002 Stage 2

Distinct from v2 multimodal work: RFC-002 Stage 2 (CLI's own
`PtySessionProvider` registering against axon's
`SessionRegistry` at boot) is blocked on a binary topology
decision (`AXON-RFC-002-stage-2-topology-addendum.md`, Path C
recommended).

If Stage 2 ever moves forward, it will touch:

- Single-binary vs cross-process registration
- `BuiltinPtySessionProvider` deletion (auto-registered today)
- CLI's `fleet.pty_session_*` deprecation

These are unrelated to v1's data-plane protocol contract; v1
ships and remains binding regardless of which Stage 2 path is
eventually taken.

### Audit findings deferred (non-blocking)

From the round-2 audit (`AXON-RFC-003-code-review.md`), still
open with no planned fix date:

- **P1-5** mutex held across `prost::Message::encode_to_vec`
  in the bridge — fine for PTY-scale frames; will become a
  bottleneck at media bitrates. Defer until media ships.
- **P2-1..P2-11** (eight design smells from round 1, three
  from round 2) — opportunistic fixes, not v1 blockers.
- **P3-1..P3-9** (nine observations) — no action required.

The audit document itself is the open-issues source of truth;
this note only acknowledges the deferrals.

---

## §4 — How to use this note

A reviewer asking "what is v1?" reads:

1. **This file** for the headline state and audit trail.
2. `AXON-RFC-003-invokebidi-protocol.md` for the binding
   normative text (§1–§9 of the spec).
3. `AXON-RFC-003-code-review.md` for the round-1 + round-2
   audit findings and severity rationale.

A reviewer asking "is implementation X conformant?" reads:

1. The spec's §8 conformance checklist.
2. `core/runtime-rs/src/tests/rfc003_bidi_v1_conformance.rs`
   for the kernel-side reference test set.
3. The implementation's own conformance suite (e.g. for a
   Go SDK, the cross-language signature matrix tests in
   `EasyNet-Axon/sdk/go/`).

A reviewer asking "what's not in v1?" reads:

1. §3 of this note (mirror image of the spec deferrals).
2. The audit document for severity-tagged open issues.

This note is the **single entry point** for the v1 audit
surface. It will not be updated except for errata; the v1 state
it describes is the v1 state until v2 (or an explicit v1.x
amendment) supersedes it.

---

End of verification note.
