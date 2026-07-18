# AXON-RFC-006 — Transition Receipts for Ability-State Semantics

**Status**: v0 — for approval.
**Date**: 2026-04-28
**Author**: Claude (under Silan.Hu architectural authority)
**Scope**: a minimal, additive upgrade to receipt semantics. RFC-003
v1 wire format, envelope schema, FFI surface, chain math, and
admission gate are unchanged. This RFC adds two things: (a) a
three-way classification of every Ability into Query / Transition /
Stream, and (b) for the Transition class only, a small set of fields
in the receipt body and the descriptor that turn a receipt from
"this invocation happened" into "this state object went from a
verifiable pre-state to a verifiable post-state under a declared
transition." Nothing more.

---

## §0 — Why this RFC exists

A receipt today (RFC-003 v1) proves: "callee X, at time T, observed
this signed envelope, ran the handler, and signed this body." It
does **not** prove anything about the state of the system after the
call. For pure read calls (`fleet.list_agents`, `meta.describe`)
that's fine — there is no state. For long-lived stateful objects
(a media call, a session, a job), the receipt is currently a log
line, not a state certificate.

This RFC closes that gap for the smallest viable surface — only
abilities explicitly declared `class = "Transition"` carry the
extra fields. Query and Stream abilities are unchanged. Existing
abilities default to Query and need no migration.

This is **not** an attempt to model the entire system as a state
machine. It is the minimum scaffolding needed for one immediate
consumer: the `media.call.*` cluster blocked by RFC-005 §2.2.

---

## §1 — Three ability classes

Every advertised AbilityDescriptor MUST declare one `class` value:

```
Query       — no state mutation; receipt proves observation.
Transition  — state mutation; receipt proves pre/post transition.
Stream      — long-lived observation or bidirectional transport;
              receipt proves stream lifecycle, not every frame.
```

Worded constraints:

- A Query handler MUST NOT mutate any state object reachable by a
  state_key. (Caches, log entries, in-memory counters used purely
  for the handler's own bookkeeping are not "state objects" in this
  sense — only objects that some Transition ability declares as its
  `state_type`.)
- A Stream handler may carry state in its session object (e.g. an
  open connection, a subscription cursor), but that state is the
  stream's own lifecycle. Per-frame payloads are not transitions.
  See TR-INV-6.
- A Transition handler MUST advance `state_version` for the named
  `(state_type, state_key)` pair by exactly one, atomically (TR-INV-9).

Multi-target transitions (one ability that can land in any of
several post-states) MUST be split into multiple distinct ability
descriptors, each with its own `transition_id` and single `to_state`.
This keeps receipts unambiguous.

---

## §2 — Schema additions

Two schema files carry the binding form. This section is the
human-readable summary; the JSON Schema files are normative.

- Receipt body delta: `schemas/receipt/transition_receipt_body.schema.json`
- Descriptor delta: `schemas/descriptor/transition_descriptor.schema.json`

Existing AbilityDescriptor fields (plan v4.1.1 §1.6: `name`,
`owner_agent_ura`, `visibility`, `scope_subjects[]`, `scope_agents[]`,
`source`, `schema_summary`, `hints`) are unchanged.

### §2.1 — AbilityDescriptor delta

```toml
# Always required (every descriptor):
class         = "Transition"           # | "Query" | "Stream"

# Required iff class == "Transition":
transition_id = "media.call.join"      # stable, dotted, deployment-invariant
state_type    = "media.call"           # the typed state object family
state_key_arg = "call_id"              # name of input arg holding the key

[transition]
from_states = ["CREATED", "ACTIVE"]    # one or more legal source states; ["*"] = any
to_state    = "JOINING"                # exactly one target state
```

### §2.2 — Receipt body delta (Transition only)

```json
{
  "transition_id":      "media.call.join",
  "transition_name":    "media.call.join",
  "attempt_id":         "01J0SAMP1E...",
  "state_type":         "media.call",
  "state_key":          "call_01HXXX",
  "pre_state_hash":     "sha256:abc...",
  "post_state_hash":    "sha256:def...",
  "pre_state_version":  7,
  "post_state_version": 8,
  "state_version":      8
}
```

These ride **inside the existing receipt body**. RFC-003 v1
envelope, signature input, chain HMAC math, and FFI shape are
unchanged.

`state_version` is a convenience alias for `post_state_version`.
The implementation MUST reject receipts where they differ.

### §2.3 — What a state_hash is

A state_hash is `sha256:<hex>` of the canonical encoding (CBOR
deterministic per RFC-8949 §4.2.1, the same encoding used elsewhere
in axon for signature inputs) of the typed state object as the
handler observed it. The schema for each `state_type` is owned by
the same module that owns the Transition abilities for that
state_type — RFC-006 does not prescribe state object schemas. It
only requires that whichever encoder the handler picks be (a)
deterministic and (b) stable.

For chain linkage (TR-INV-4) to hold across handlers, all handlers
that operate on the same `state_type` MUST agree on the encoding
within a deployment. This is a per-state_type invariant the owning
module MUST pin.

---

## §3 — Worked example: `media.call`

Illustrative only — the actual landing of these abilities is
RFC-005 §2.2 work. RFC-006 itself ships no media abilities.

### State space

```
state_type: media.call

states:
  CREATED        — call object exists; no participants joined
  JOINING        — at least one participant has joined; SDP not yet exchanged
  NEGOTIATING    — SDP offer/answer in flight
  ACTIVE         — bidirectional media flowing
  ENDED          — terminal; no further transitions allowed
```

### Six descriptors

```
abilities/media/call_create.ability.toml
  class         = "Transition"
  transition_id = "media.call.create"
  state_type    = "media.call"
  state_key_arg = "call_id"
  [transition] from_states = ["*"]                        to_state = "CREATED"
  # "from ∅" is encoded as ["*"] with handler responsibility to
  # reject pre_state_version > 0; create is the only legal way to
  # bring a call into existence.

abilities/media/call_join.ability.toml
  class         = "Transition"
  transition_id = "media.call.join"
  state_type    = "media.call"
  state_key_arg = "call_id"
  [transition] from_states = ["CREATED", "ACTIVE"]        to_state = "JOINING"

abilities/media/call_set_description.ability.toml
  class         = "Transition"
  transition_id = "media.call.set_description"
  state_type    = "media.call"
  state_key_arg = "call_id"
  [transition] from_states = ["JOINING", "NEGOTIATING"]   to_state = "NEGOTIATING"
  # NEGOTIATING → ACTIVE is a separate descriptor (split per §1
  # multi-target rule); naming convention TBD by media owner.

abilities/media/call_end.ability.toml
  class         = "Transition"
  transition_id = "media.call.end"
  state_type    = "media.call"
  state_key_arg = "call_id"
  [transition] from_states = ["*"]                        to_state = "ENDED"

abilities/media/call_get.ability.toml
  class         = "Query"
  # No transition_id, state_type, state_key_arg, [transition].
  # MAY informally document state_type for discovery but MUST NOT
  # emit a transition receipt body.

abilities/media/call_watch_transport_events.ability.toml
  class         = "Stream"
  # Stream session lifecycle (open / close / error) is the receipt
  # surface; per-frame transport events are payload, not transitions.
```

### What this buys

A verifier with the receipt for `media.call.create` followed by
the receipt for `media.call.join` on the same `call_id` can:

1. Read `create.post_state_hash`. Read `join.pre_state_hash`.
   Verify they are equal (TR-INV-4).
2. Read `create.post_state_version = 1`. Read `join.pre_state_version = 1`,
   `join.post_state_version = 2`. Verify they line up (TR-INV-9).
3. Read `join.transition_id = "media.call.join"`. Look up the
   descriptor. Confirm `from_states` includes `CREATED`. Confirm
   `to_state == "JOINING"` matches whatever the post-state encodes.

That gives an external auditor a state-transition record for the
call without trusting the SFU's logs.

---

## §4 — What this RFC is NOT

- **Not a proof of correctness.** A transition receipt is a *signed
  transition claim*: callee asserts it observed pre_state, applied
  the declared transition, produced post_state, and signed all of
  it. A malicious or buggy callee can sign a false claim. RFC-006
  pins the claim's *shape* and *verifiability*, not the callee's
  honesty. For Byzantine settings, layer attestation / quorum on
  top.
- **Not retroactive.** Existing abilities that have not declared
  `class` default to Query — they continue to work and continue to
  emit only standard receipts. There is no migration deadline.
- **Not a new wire format.** Receipt body is a free-form JSON
  object today; transition receipts add named keys to that object.
  Envelope, signature input, chain HMAC, FFI ABI, payload transfer
  — all unchanged.
- **Not a state machine framework.** This RFC says nothing about
  *how* the handler stores state, *how* the state encoding is
  versioned, *what* a state_type's schema looks like. Those are
  state-type-owner concerns. RFC-006 only fixes the receipt and
  descriptor shape.
- **Not a replacement for `hints`.** Existing
  `hints.{read_only, destructive, idempotent, streaming_only}` stay.
  They are advisory; `class` is binding.

---

## §5 — Migration and compatibility

### §5.1 — RFC-003 v1 invariants are preserved

Inventory of v1 invariants vs RFC-006:

| RFC-003 v1 invariant family | RFC-006 effect |
|---|---|
| INV-1..10 (envelope / signature) | Untouched. RFC-006 fields live in receipt body, not envelope. |
| FFI-INV-1..9 (FFI ABI shape) | Untouched. New fields are body bytes, opaque to FFI. |
| BP-INV-1..6 (backpressure) | Untouched. Per-frame semantics unchanged. |
| TERMINAL-INV (terminal receipt rules) | Untouched. A Failed transition MUST NOT emit a transition receipt body — failure receipts use the existing terminal shape. |
| Chain HMAC math | Untouched. Body is included in the existing signed body span. |

If any conformance script for RFC-003 v1 fails after RFC-006 lands,
the implementation is wrong, not the RFC.

### §5.2 — RFC-005 status update

RFC-005 §1 row for #128 changes from:

```
| #128 | … | … | … | … | needs new namespace |
```

to:

```
| #128 | … | … | … | … | blocked by RFC-006 transition semantics |
```

A new paragraph is appended to RFC-005 §2.2 noting the seven
proposed `media.*` abilities are reclassified as **4 Transition + 1
Query + 1 Stream** (call_set_description's NEGOTIATING → ACTIVE
target gets split into a second Transition descriptor; that's the
seventh — see §3 above for the split rule).

#122, #184 dissolved verdicts in RFC-005 are unaffected.

#185 parked verdict in RFC-005 is unaffected; its eventual
`session.mirror` / `session.handoff` abilities will, when they
arrive, themselves be Transition under RFC-006.

### §5.3 — Default classification for legacy descriptors

Every existing AbilityDescriptor in `EasyNet-Cli/ability-descriptors/system/`
is implicitly `class = "Query"` until it explicitly declares
otherwise. The descriptor loader MUST treat absent `class` as
`"Query"` and MUST reject Query descriptors that emit transition
receipt bodies (TR-INV-5).

The four PTY-cluster descriptors (`fleet.pty_session_create`,
`fleet.pty_session_close`, `fleet.pty_session_attach`,
`fleet.session_attach`) are good candidates for opting in to
Transition / Stream classification in a follow-up — they
genuinely advance session state — but RFC-006 does not require
that migration. It is purely additive. RFC-005 #122 stays
dissolved; this is a separate, optional refinement.

---

## §A — Binding constraints (TR-INV)

Future revisions MUST satisfy every item below. Each row names the
exact verification location. None of these are doc-only — every
TR-INV has a CI script or a test case.

| # | Rule | Verifier |
|---|---|---|
| TR-INV-1 | Every descriptor with `class = "Transition"` MUST declare `state_type`. | `scripts/check-transition-has-state-type.sh` (greps every TOML where class=Transition; fails if state_type absent) |
| TR-INV-2 | Every descriptor with `class = "Transition"` MUST declare `[transition]` with `from_states` (≥1) and exactly one `to_state`. | `scripts/check-transition-has-pre-post.sh` (TOML grep + JSON Schema validate against `transition_descriptor.schema.json`) |
| TR-INV-3 | Every successful transition receipt body MUST include `pre_state_hash` and `post_state_hash` (and the rest of §2.2's required fields). | `tests/receipt_transition_carries_hashes_test.rs` (e2e: invoke media.call.create through a fake handler, assert all required fields present per `transition_receipt_body.schema.json`) |
| TR-INV-4 | For two transition receipts on the same `(state_type, state_key)`, if one ordered immediately after the other in the daemon-side state log, then receipt(n+1).pre_state_hash == receipt(n).post_state_hash. | `tests/receipt_chain_links_state_test.rs` (invoke create→join, assert pre/post hashes line up) |
| TR-INV-5 | Query and Stream abilities MUST NOT emit a receipt body containing transition fields. | `tests/query_stream_no_transition_body_test.rs` (invoke meta.describe and a Stream ability, assert receipt body has none of: transition_id, pre_state_hash, post_state_hash, pre_state_version, post_state_version, state_version, state_type, state_key, attempt_id) |
| TR-INV-6 | Stream session state MUST live separately from per-frame payload. A Stream's lifecycle receipt MAY use a state_type for the session object, but per-frame BinaryChunk / data frames MUST NOT carry transition receipt fields. | `tests/stream_session_state_separate_test.rs` (open a Stream ability, send N data frames, assert no frame body contains transition fields; the closing receipt MAY but is not required to) |
| TR-INV-7 | `transition_id` MUST be stable across deployments and SDK versions. `transition_name` MAY be a human-readable alias and MUST NOT be used for routing or verification. | `scripts/check-transition-id-stable.sh` (grep all TOMLs, build a set of transition_ids; compare against a checked-in `transition_ids.lock` file; fail on rename without explicit lock-file update) |
| TR-INV-8 | If `state_key_arg` names a field absent from the invocation args, or whose value is not stringifiable, the handler MUST reject with `InvalidArgument` and MUST NOT advance state. The state_key recorded on the receipt MUST be the canonicalized string form. | `tests/state_key_arg_validation_test.rs` (invoke media.call.join with no call_id → InvalidArgument; invoke with call_id=42 → receipt has state_key="42"; invoke with call_id={} → InvalidArgument) |
| TR-INV-9 (SV-INV-1) | A Transition handler MUST atomically (a) read current state_version for `(state_type, state_key)`, (b) verify `pre_state_version` matches what the handler observed, (c) compute the new state, (d) persist with `post_state_version = pre + 1`, all in one critical section. Concurrent transitions on the same key MUST serialize. | `tests/transition_atomic_version_test.rs` (spawn two concurrent media.call.set_description on same call_id; assert exactly one wins, the loser sees a version-mismatch error, and post_state_version increments by exactly 1) |
| TR-INV-10 | `attempt_id` MUST be globally unique per attempt. Retries MUST mint fresh `attempt_id`; reuse is forbidden. | `tests/attempt_id_unique_test.rs` (run the same logical transition twice — once succeeds, once retries — assert two distinct attempt_ids on the two receipts) |

---

## §B — Approval gate

Pick:

- **Approve v0** as written. I open a follow-up task to (a) write the
  10 verifier scripts/tests, (b) patch RFC-005 per §5.2, (c) wire the
  descriptor loader to honour `class` + the new fields. No code
  before that.
- **Revise** — name the section.
- **Defer** — leave RFC-006 at v0, do not patch RFC-005, treat #128
  as still blocked but with the framing clearer.
