# AXON-RFC-005 — Ability-Expansion Map for the Four Deferred Items

**Status**: design — mapping doc, no code.
**Date**: 2026-04-27
**Author**: Claude (under Silan.Hu architectural authority)
**Scope**: re-frame the four items deferred by RFC-003 v1
verification (§3) — #122 wshandler-on-bidi-pty,
#128 conference SFU, #184 RFC-002 Stage 2,
#185 RFC-002 Stage 3 — as **ability-namespace
growth**, not protocol-layer changes.

---

## §0 — Why this is allowed (and why this doc exists)

RFC-003 v1 is binding on the data-plane protocol, FFI, and chain
math. It is not binding on the **ability registry**: plan v4.1.1
§18 explicitly enumerates the catalog and §1.6 defines the
descriptor schema, and both are designed for forward growth.
RFC-003 v1 §9.5 (the "non-goals" section) leaves new ability
descriptors untouched.

So: any of the four deferred items that can be expressed as

```
new ability descriptor (§1.6)
   → owned by a profile already in §1
   → invoked over the existing Invoke / InvokeStream / InvokeBidi
```

ships **without** touching:

- the kernel admission gate
- chain HMAC math
- FFI surface
- proto wire format
- any RFC-003 v1 invariant (INV-1..10, FFI-INV-1..9, BP-INV-1..6,
  TERMINAL invariants)

This document maps each of the four items into that envelope, so
the work becomes downstream of v1 rather than blocked by it.

The rule the user invoked: *"there is no current demand, but do
it, and treat it as ability."* — this doc is the precondition;
TOMLs + handlers come after approval.

---

## §1 — Mapping table

| # | Original framing | Re-framed as | Owner profile | Visibility | Verdict |
|---|---|---|---|---|---|
| #122 | wshandler should call PTY-bidi | route through `fleet.session_attach` ability | device | SCOPED to operator | **dissolved** — already shipped + already wired |
| #128 | conference SFU on Axon voice signaller | new `media.*` ability cluster, owned by a sfu-profile (or backend if co-located) | sfu (new) or backend | SCOPED to call participants | **blocked by RFC-006 transition semantics** |
| #184 | RFC-002 Stage 2 cross-process SessionProvider | re-projection of resource ownership as `session.*` abilities; cross-process needs are eclipsed by ability dispatch | device (PTY) / llm (chat) | SCOPED to owner | **dissolves under ability lens** |
| #185 | RFC-002 Stage 3 distributed sessions | only meaningful as ability-level mirroring (`session.mirror`, `session.handoff`); not a protocol concern | device + llm | SCOPED (admin) | **deferred — no use case until #128 ships** |

Two of four (#122, #184) **dissolve**: nothing to do.
One (#128) needs a fresh ability cluster.
One (#185) is parked behind #128.

---

## §2 — Per-item walk-through

### §2.1 — #122 (wshandler-on-bidi-pty) — DISSOLVED

**Current code already does this.** `backend/internal/handler/
terminal/wshandler.go:308` opens `InvokeBidi` against
`fleet.session_attach` with the daemon-side PTY session_id as
`initial_args`, then runs symmetric `pumpReader` /
`pumpWriter` goroutines. The ability is owned by the
device-profile and lives in `EasyNet-Cli/ability-descriptors/system/
fleet.session_attach.ability.toml` plus
`fleet.pty_session_attach.ability.toml`, with handlers under
`EasyNet-Cli/src/runtime/agents/pty_attach_ability.rs`.

Round-trip is exercised by
`EasyNet-Cli/src/runtime/agents/real_invoke_tests.rs::
real_fleet_pty_session_attach_returns_a_bidi_source`.

**Verdict**: nothing to design. #122 in the task list is
satisfied by the v0.X work already merged (the
`session_attach` + PTY bidi cluster). Recommend marking it
closed when the user re-opens the task list.

### §2.2 — #128 (conference SFU) — NEEDS NEW NAMESPACE

`backend/internal/sfuprovider/axon_ability.go:97` calls
`voice.JoinVoiceCall(...)` against `axon.VoiceSignaller`. That
interface is dead — its source comment
(`backend/internal/axon/voice.go:6-12`) reads:

> The conference SFU was the only consumer of this facade; its
> bridge-backed implementation went away with the legacy
> 16-method Client surface in P5-fix-5c… When the voice-stream
> ability lands per plan §18, this interface gets re-implemented
> on top of it.

So #128's actual shape is: **define the ability cluster that
`VoiceSignaller` will be re-implemented on top of, then rewrite
the dead interface as a thin Invoke adapter.**

Proposed cluster (per §1.6 AbilityDescriptor):

```
media.call.create
  owner_agent_ura:  <sfu-profile or backend-profile> URA
  visibility:       SCOPED
  scope_subjects:   [creating operator URA]
  source:           manifest:abilities/media/call_create.ability.toml
  schema_summary:
    input:  { mode: "p2p"|"conference", codec, max_participants }
    output: { call_id, sfu_endpoint }
  hints:    { read_only: false, idempotent: false }

media.call.join
  visibility:       SCOPED
  scope_subjects:   [participants in call_id's invite list]
  schema_summary:
    input:  { call_id, participant_id, codec, muted }
    output: { transport_session_id, sfu_offer_sdp }

media.call.set_description
  visibility:       SCOPED to call participants
  schema_summary:
    input:  { call_id, transport_session_id, side: local|remote, sdp, type }
    output: { revision_id }

media.call.watch_transport_events
  visibility:       SCOPED to call participants + sfu provider
  schema_summary:
    input:  { call_id, from_sequence, max_events, timeout_ms }
    output: { events[] }                  // streamed via InvokeStream
  hints:    { streaming_only: true, read_only: true }

media.call.get_transport_session
  visibility:       SCOPED to call participants
  schema_summary:
    input:  { call_id, transport_session_id }
    output: { session: {participant_id, local_description, remote_description, ...} }
  hints:    { read_only: true }

media.call.get
  visibility:       SCOPED to call participants
  schema_summary:
    input:  { call_id }
    output: { call: {state, mode, participants[], ...} }
  hints:    { read_only: true }

media.call.end
  visibility:       SCOPED (creator OR admin)
  schema_summary:
    input:  { call_id, reason }
    output: { ack }
```

Owner-profile decision: **defer to implementation time.** Two
shapes are viable:

1. **sfu-profile (new)** — co-located with the conference
   handler in the backend process, signed by a hosted Agent
   the backend mints in `local-agents.json`. Cleanest
   ontology fit.
2. **backend-profile** — sfu Abilities advertised by the
   existing `01BAK` Agent. Less plumbing, but conflates
   media routing with aggregation.

Recommend (1) when implementation lands, but the mapping doc
doesn't lock it. The cluster shape above is independent of
the choice.

`VoiceSignaller` rewrite (post-cluster): each method
constructs an `InvokeRequest` against the matching `media.*`
ability, with backend `SessionAuthority` for backend-mediated
sessions or a true user-signed `DelegationProof` when the user key
signs directly. The wire format and signing path are unchanged.

**Verdict**: needs **new namespace `media.*`** (7 abilities)
plus an owner-profile decision. No protocol change. No FFI
change. No kernel change.

**Update (2026-04-28, post RFC-006 v0)**: the seven abilities
listed above are reclassified per RFC-006 §1 as **5 Transition + 1
Query + 1 Stream**:

- Transition: `media.call.create`, `media.call.join`,
  `media.call.set_description` (split into JOINING|NEGOTIATING →
  NEGOTIATING and NEGOTIATING → ACTIVE per RFC-006 §1 multi-target
  rule), `media.call.end`. The split takes the original
  `set_description` count from one to two, so the cluster grows
  from seven to eight descriptors.
- Query: `media.call.get`, `media.call.get_transport_session`.
- Stream: `media.call.watch_transport_events`.

Implementation of #128 is now **blocked by RFC-006 v0 approval** —
the descriptor loader must honour `class` + the new
`state_type` / `state_key_arg` / `[transition]` fields before any
`media.*` TOML lands. Owner-profile decision (sfu-profile vs
backend-profile) is unchanged.

### §2.3 — #184 (RFC-002 Stage 2: cross-process SessionProvider) — DISSOLVES

Stage 2's premise was that a Rust `SessionProvider` registered
in one process needs to be reachable from another process via
`SessionRegistry`. Under the ability lens, "session ownership"
is **already** projected as ability surface:

| Resource | Ability that owns it |
|---|---|
| PTY session | `fleet.pty_session_create/close/attach` (shipped) |
| LLM chat session | `session.create/list/resume/close` (shipped per §18) |
| Bidi pipe | `fleet.session_attach` (shipped) |
| Voice transport | `media.call.*` (per §2.2 above) |

A "cross-process" caller does not need a cross-process
`SessionProvider`. It needs to invoke the right ability against
the right Agent URA, and let the LocalAgentCatalog →
RealmDirectory → gRPC routing path (per plan v4.1.1 §1.2) do
the rest. That path is the canonical solution; cross-process
SessionProvider was a workaround for the era when the routing
path didn't exist.

**Verdict**: #184 is moot. Recommend closing as
"superseded by ability dispatch + LocalAgentCatalog (plan
v4.1.1 §1.2)."

### §2.4 — #185 (RFC-002 Stage 3: distributed sessions) — PARKED

Stage 3 was about session migration / mirroring across hosts.
Under the ability lens this becomes:

```
session.mirror   (owner: device or llm; SCOPED admin)
session.handoff  (owner: device or llm; SCOPED admin)
```

But these have **no consumer** today. The PTY use case is
host-local; the LLM use case is one-Agent-per-host; #128's
conference SFU explicitly centralises media on a single
provider. Until a real use case appears, designing the
descriptors is speculative.

**Verdict**: park behind #128. When #128 ships and the SFU
provider needs failover, lift the §2.3 `session.*` cluster
into mirror/handoff variants. Until then, no mapping work.

---

## §3 — What dissolves vs what's left

```
#122  →  DISSOLVED       (already shipped as fleet.session_attach + pty_session_*)
#184  →  DISSOLVED       (superseded by ability dispatch path)
#128  →  ABILITY WORK    (define media.* cluster, owner-profile decision, rewrite VoiceSignaller as Invoke adapter)
#185  →  PARKED          (no use case until #128's SFU needs failover)
```

So of four items, **one** (#128) becomes real downstream work.
Estimated landing surface:

- **Cli side**: 7 ability TOMLs under `EasyNet-Cli/abilities/
  media/`, 7 handler stubs under `src/runtime/agents/
  media_*_ability.rs`, registration in
  `src/services/control/server.rs`.
- **Backend side**: rewrite
  `backend/internal/axon/voice.go::VoiceSignaller` methods to
  call `RealClient.Invoke` against `media.*`; delete the
  retirement-notice TODO in
  `backend/internal/svc/servicecontext.go` once
  `sfuprovider.RegisterAndServe` is wired again.
- **Owner-profile decision**: sfu-profile vs backend-profile
  (one TOML + one entry in `local-agents.json` schema if
  sfu-profile).

No proto changes. No FFI changes. No kernel changes. No
RFC-003 v1 invariant moves.

---

## §4 — Implementation order (when user approves)

1. **Approve owner-profile choice** for `media.*` (sfu-profile
   recommended; backend-profile acceptable).
2. **Land the seven `media.*` ability TOMLs** (Cli side, no
   handlers yet — registry + descriptor publish only).
3. **Land seven handler stubs** (Cli side) that return
   `unimplemented!` until the SFU is wired.
4. **Rewrite `VoiceSignaller`** as a thin Invoke adapter over
   `media.*` (Backend side). At this point it type-checks and
   is callable but the handlers still error.
5. **Port the SFU body** from `axon_ability.go` into the
   handlers (move the pion/webrtc logic from
   backend/sfuprovider into a Cli-side handler under
   `media.call.watch_transport_events` etc.) — OR, if SFU
   stays in backend, keep the body where it is and have the
   handlers proxy back via a local socket. Latter is uglier
   but lets us ship faster.
6. **Smoke test**: `easynet call create --mode conference`
   end-to-end through the new ability surface. No new test
   matrix; the existing conference-call path becomes the
   conformance pin.
7. **Mark #128 closed.** Mark #122 + #184 closed as dissolved.
   Leave #185 open with a pointer to "lift §2.3 abilities into
   mirror/handoff variants when SFU failover lands."

---

## §5 — Approval gate

Pick one:

- **Approve** §1 mapping + §4 implementation order. I begin
  step 1 (owner-profile decision request) on next turn.
- **Approve mapping, defer implementation** — close #122 + #184
  as dissolved in the task list, leave #128 + #185 open with
  pointers to this doc, no code yet.
- **Revise** — name the section to revise (e.g. owner-profile
  shape, `media.*` schema fields).

No code is written until you reply.
