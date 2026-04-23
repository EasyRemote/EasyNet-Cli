# PR-10 — EAL `loop` executor (loop-only)

**Status:** Stage 1 & 2 merged · Stage 3 in progress · **Date:** 2026-04-23 · **Owner:** Silan Hu

## What PR-10 is

Three stages implementing `docs/rfc/eal-control-flow-v1.md`:

1. **Stage 1 — IR node.** `IrStep::Loop(IrLoop)` added to `src/eal/ir.rs`. Merged (`0a10ced`).
2. **Stage 2 — parser.** `loop { body { } verify { } }` grammar accepted. Merged (`9349378`).
3. **Stage 3 — executor.** In-process sequential loop runner with `max_iters` bound, `verify.done: bool` termination, typed `LoopExhausted` / `VerifyMalformed` errors. **In progress.** Tracked under task #38.

After Stage 3 merges, **PR-10 is done**. No Stage 4.

## What PR-10 is NOT

- **No `chat { }` block form.** Descoped to `docs/open-questions/discuss-eal-block.md`. Reason: naming collision with the AXIOM `chat` ability name, and no gallery mission consumes a block-form conference today. Will revisit under the name `discuss { }` if a gallery mission names the need.
- **No `handoff { }` block form.** Deleted outright. Reason: zero protocol ground, zero consumer, expressible in two flat EAL statements today.
- **No `services/loop_exec/` module.** Tier B deferred per `docs/open-questions/does-easynet-need-a-local-ws-control-plane.md`. Stage 3 is in-process only; the RFC §7 daemon-online column is a contract for a future implementation with a consumer.
- **No `mission loop` CLI verb.** Not requested; `easynet mission run <file>.eal` already runs any mission that contains a `loop` block.
- **No `permit` top-level.** Per `does-easynet-need-a-local-ws-control-plane.md`, `permit` as an EAL construct waits on PR-10 Stage 3 + a customer; as a local WS protocol it's further deferred.

## Stage 3 scope (for the in-progress PR)

### In scope

- Planner: `Statement::Loop` → `IrStep::Loop(IrLoop)` lowering in one-pass compile.
  - Validate `max_iters ∈ [1, 32]` (RFC §3.1).
  - Validate `body` non-empty and `verify` non-empty.
  - Validate last statement of `verify` is a call (RFC §4.4 — so it can produce `done: bool`).
  - Reject nested loops (RFC §4.2 v1).
  - Hermetic scope: outer mission bindings not visible inside `body` / `verify`; inner bindings not leaked out except `<name>.result`.
- Interpreter: `execute_loop` runs body+verify sequentially per iteration, inspects verify's final call output for top-level `done: bool`, terminates on `done == true`, aborts with `LoopExhausted` if `max_iters` hit without `done`.
- Interpreter: `VerifyMalformed` hard abort on missing / non-boolean `done` field, or verify final output absent.
- Interpreter: Loop iterations each go through the same dispatch path as flat Calls; each call counts against `MAX_AGENT_DEPTH` but iterations do not stack depth (each iter is depth 1, not depth N).
- Interpreter: outer phase walk collapses to one-step-per-phase source order when any top-level item is a `Loop`; pure-Call missions retain the pre-PR-10 parallel-when-independent scheduling.

### Out of scope

- Timeline event types for loop iteration (optional; if emitted, match PR-7 Commit 2's shape).
- Parallel loop iterations (RFC §6 v1: single-phase).
- Chat / Handoff block execution (descoped / deleted — see §"What PR-10 is NOT").
- RFC-driven removal of `IrStep::Chat` / `IrStep::Handoff` scaffold types from `src/eal/ir.rs`. That is a **separate follow-up PR**, tracked below.

### Tests Stage 3 must include

1. `loop_max_iters_out_of_range_rejected` — `max_iters: 0` and `max_iters: 33` are compile errors (§5.1).
2. `verify_without_done_bool_aborts` — verify returning an object without `done`, or with `done: "yes"`, aborts as `VerifyMalformed`.
3. `loop_exhausts_with_typed_error` — `max_iters: N` with verify always returning `done: false` aborts as `LoopExhausted { name, max_iters: N }`.
4. `loop_with_body_dispatch_does_not_nest_depth` — a 4-iter loop dispatching to one agent per iter stays at depth 1 (no `MAX_AGENT_DEPTH` trip).
5. `loop_terminates_on_done_true` — happy path: verify returns `done: true` on iteration K ≤ max_iters, loop exits and `<name>.result` binds the verify call's output.
6. `nested_loops_rejected_at_compile_time` — RFC §4.2 v1.
7. `loop_last_verify_is_not_call_rejected` — RFC §5.1.
8. `hermetic_scope_outer_binding_not_visible_inside_body` — a `body` step referencing an outer `let` binding is a compile error.

Budget target: 150–250 lines. Hard ceiling: 400 lines per the Stage 3 greenlight.

## Follow-up PRs (not this one)

### PR-10.1 — remove Chat/Handoff scaffold from IR and parser

Once Stage 3 merges and operators have confirmed the loop-only path is what they want, a separate PR removes the `IrStep::Chat` / `IrStep::Handoff` / `AST Statement::Chat` / `AST Statement::Handoff` / parser `chat` / `handoff` keyword handling. The scaffold remains in the tree after Stage 3 only for on-disk compatibility with missions compiled during the Draft-revision window of the RFC (short window; internal only — no external consumer holds those artefacts).

This PR is small (~200 lines of deletions + bail-message rewrites). It is *not* part of Stage 3 to keep the Stage 3 diff focused on the one thing the Stage 3 greenlight authorised.

### Beyond

- OpenCode driver (#9) — awaits customer.
- FakeAgentClient test infra (#36) — scheduled at PR-7 merge once Axon WIP unblocks.

## Plan-audit clause (strengthened 2026-04-23)

**Original clause (weaker):**

> Any unscheduled entry must point at a customer, issue, RFC, AXIOM convention, or failed-mission trace; otherwise goes to open-questions or gets dropped.

**Loophole.** An RFC can be authored by the plan author alone. Under the original wording, an author could write any RFC and then cite it as the evidence for a scheduled entry, auto-certifying their own judgement. The `chat { }` and `handoff { }` sections of the RFC's Draft revision are the historical example of this loophole firing: both were in the plan, both cited "the RFC," neither had an external consumer.

**Strengthened clause:**

> Any scheduled entry must point at one of:
> - an external customer request with a named owner,
> - a GitHub/tracker issue,
> - a merged RFC **with at least one consumer** (gallery mission, code caller, integration test, or documented cross-repo dependency),
> - an AXIOM convention explicitly named and citable,
> - a failed-mission trace with a file reference.
>
> An RFC that lacks a consumer is itself treated as an unscheduled entry and must be validated against this rule. The phrase "the RFC says we should" is not, by itself, evidence of ground — the RFC must either (a) be AXIOM, or (b) point at a consumer.

**How this clause bites going forward.** Before any new entry lands on the plan, the author must name the consumer concretely and by file path. If the only consumer is the proposing RFC itself, the entry goes to `docs/open-questions/` with a trigger that names what a real consumer would look like. This is the discipline that moved `chat` / `handoff` off the plan and keeps `loop` on it.

## Log

| Date       | Event                                                                                     |
|------------|-------------------------------------------------------------------------------------------|
| 2026-04-22 | Plan-audit clause (original form) introduced alongside the 不排期堆积 list review.        |
| 2026-04-23 | Stage 1 merged (`0a10ced`).                                                               |
| 2026-04-23 | Stage 2 merged (`9349378`).                                                               |
| 2026-04-23 | Stage 3 greenlit (loop-only, 150–400 line budget, 8-test audit-hook floor).               |
| 2026-04-23 | RFC rewritten: loop-only, `handoff` deleted, `chat` → `discuss` descoped to open-question. |
| 2026-04-23 | Plan-audit clause strengthened to close the author-authored-RFC self-certification loophole. |
