# Open Question — Does EAL need a `discuss { }` block form?

**Status:** Open · **Trigger-based revisit** · **Owner:** Silan Hu · **Date:** 2026-04-23

## The question

An earlier Draft of `docs/rfc/eal-control-flow-v1.md` proposed a
`chat { participants, max_turns, topic, visibility }` block form
for multi-agent conferences inside EAL missions. The approved RFC
removed it — see RFC §10 for the audit.

What remains open: **should EAL eventually grow a block form
(renamed `discuss { }` to avoid the `chat` ability-name collision)
that mirrors the existing `easynet discuss` CLI verb at the
language level?**

## Why this is not in the plan

The plan-audit clause requires every scheduled entry to point at a
named consumer. For this block form:

1. No gallery mission today invokes a multi-agent conference as an
   EAL block step. The case01-ARIS missions that motivate `loop`
   (`auto-review-loop.eal`) compose conferences at the mission
   *boundary*, not inside a single EAL mission.
2. `easynet discuss` exists at the CLI verb layer
   (`src/facade/cli/discuss.rs`) and is the current operator
   interface. Shipping the same capability in two layers without
   a consumer driving the choice would be premature.
3. `chat` is already the canonical single-agent invocation ability
   name (`agent.chat(prompt: ...)`) across the system. Using the
   same token at two semantic layers is a naming trap; if this
   block form lands, it must be named `discuss { }` — this is the
   first design decision the trigger will force.

## What would move this to a plan item

A gallery mission (or a customer request pointing at one) where the
block form is *necessary*, not just convenient. Concrete shapes
that would qualify:

1. A multi-agent conference **nested inside a `loop`'s verify
   block** — the conference's transcript becomes the termination
   predicate's input. The CLI verb cannot compose this because
   the conference needs to run per-iteration from inside the
   mission's IR, not as a separate command invocation.
2. A multi-agent conference whose **transcript binding is consumed
   downstream by another EAL step** via `input_refs`. The CLI
   verb returns a transcript to stdout; if a subsequent EAL step
   needs to bind that transcript into its args, the block form is
   the clean path. (Today a caller can `easynet discuss … >
   transcript.txt` and pass the path to the next mission, but
   that moves data through the filesystem and breaks the
   mission's proof-carrying-plan property — the IR no longer
   describes the data flow.)
3. A mission where the conference **itself has preconditions that
   must be re-evaluated each run** (e.g. the participant list
   depends on an upstream step's output). The CLI verb takes a
   fixed `--agents` list at invocation time.

Any one of these triggers a revisit. Without one, the block form
stays hypothetical.

## What was considered and rejected

- **"Build it now, missions will use it later."** This is the ACP
  pattern the repo has elsewhere rejected. The EAL grammar is a
  user-facing surface; adding a block form and then hoping someone
  composes a mission around it commits to syntax we cannot
  gracefully remove if the uses don't materialise.
- **"Keep the RFC's `chat { }` section and just rename."** Rename
  alone does not add a consumer. The naming conflict is a symptom
  of the lack of grounding, not the cause.
- **"Ship a parser-level `discuss { }` that lowers to a sequence
  of `agent.chat(...)` calls today."** Possible, but the
  translation is lossy: fan-out semantics (`fan_out` vs
  `round_robin`) and the transcript binding need runtime logic
  the flat-call lowering does not express. Doing the lowering
  right is the block form we said we would not build.

## What this question does *not* block

- **PR-10 Stage 3** — Loop executor. `loop` is self-contained;
  removing `chat`/`handoff` narrows the RFC but does not alter
  `loop`'s semantics.
- **`easynet discuss` CLI verb** — unchanged. Multi-agent
  conferences at the command layer continue as today.
- **Frontend conference UI** — unchanged.

## If it becomes a plan item

Rough shape (not a spec — revisit when triggers fire):

- Name: `discuss { }` (not `chat { }`; see §10 of the approved RFC
  for the naming-collision rationale).
- Grammar: participants list, `max_turns`, optional `topic`,
  optional `visibility: fan_out | round_robin`, optional
  `transcript_binding`.
- IR: `IrStep::Discuss { … }`. Rename and reshape the scaffold
  `IrStep::Chat` variant (currently retained for on-disk compat
  with Draft-revision traces) or delete and re-add.
- Semantics: bounded (`max_turns * participants.len()` static call
  count, like `loop`'s bound). Every participant invocation goes
  through `run_mission_inproc` — no runtime bypass.
- Parser changes: `discuss` becomes a reserved keyword; `chat`
  at block-form position stays a parse error pointing at this
  doc.
- Gallery: the triggering mission lands in the same PR that
  ships the block form, so the feature ships with its consumer.

## Log

| Date       | Event                                                                          |
|------------|--------------------------------------------------------------------------------|
| 2026-04-23 | Opened as part of PR-10 RFC narrowing. Chat block dropped from approved RFC.   |
| —          | Revisit: **trigger-based**, when a gallery mission names the block form as a need. |
