# RFC: EAL Control Flow v1 — `loop` (only)

| Field | Value |
|-------|-------|
| Status | **Approved — `loop` only; `discuss` / `handoff` descoped (see §10)** |
| Scope  | EAL language + Mission IR v2 (additive) — `loop` block form |
| Motivates | `gallery/case01-aris/missions/auto-review-loop.eal` (4-round unrolled) |
| Closes (partially) | Ontology S13 deferred item #1 "EAL loops/conditionals" |
| Supersedes | The `chat { }` and `handoff { }` sections of this RFC's Draft revision — see §10 for the audit that removed them |

## 1. Why this RFC

The gallery contains a mission (`auto-review-loop.eal`) that today
hand-unrolls a three-call review/experiment/rebut cycle four times,
because EAL has no `loop` form. The mission's audit trail therefore
shows the same three calls × 4 repetitions with no marker that the
author's intent was "iterate until reviewer approves." The unrolled
form cannot express *early termination* when the reviewer signs off
in iteration 2 — every run pays the full four-round cost.

A declarative `loop { body { … } verify { … } }` form fixes both
problems: the IR records the iteration intent (auditable before
execution per ontology §5 "mission as proof-carrying plan"), and
runtime termination is driven by the `verify` block's explicit
`done: bool` predicate.

This RFC scope is **`loop` only**. An earlier Draft revision of this
RFC also proposed `chat { }` and `handoff { }` block forms. Those
were removed in the §10 audit — see that section for the specific
ground each one failed on.

## 2. Non-goals

- Turing-complete EAL. We add bounded iteration with an explicit
  termination predicate; we do not add `goto`, recursion, or user
  function definitions.
- Arbitrary conditionals. `if / else` is deferred to a follow-up RFC —
  `loop { ... verify { ... } }` subsumes the dominant "iterate until
  good" use case without introducing boolean expression evaluation
  into the IR.
- Changes to EAL's data flow model. `input_refs` and `output_binding`
  remain the only way data crosses step boundaries.
- Runtime semantics for the SDK. Everything in this RFC must work
  against the existing `mission_runs::run_mission_inproc` single entry
  point. Services-layer (`services/{loop_exec}/`) is an
  optimization — see §7.
- Multi-agent conference primitives (`chat` / `discuss` / `handoff`
  blocks). Descoped per §10 — the CLI verb `easynet discuss`
  already orchestrates multi-agent conferences at the command layer;
  an EAL block form waits on a mission that needs it.

## 3. Surface syntax

```eal
mission "example" {

  loop "review-cycle" max_iters: 4 {
    body {
      let review = reviewer.review(artifacts: "src/") timeout 600
      let fixes  = researcher.experiment_bridge(plan: review.output) timeout 1200
    }
    verify {
      reviewer.rule_on_rebuttal(rebuttal: fixes.output) timeout 300
    }
  }
}
```

### 3.1 `loop` block

Required attributes: `max_iters: <N>` (compile-time integer ≥ 1, ≤ 32).
Optional name (free-form string) for telemetry.

`body { ... }` is a statement list evaluated each iteration. `verify
{ ... }` is a statement list evaluated after each iteration; if its
final call's `output` is non-empty AND its top-level boolean field
`done` (see §4.4) is true, the loop terminates. Otherwise the loop
re-enters `body`. If `max_iters` is hit without `done: true`, the
loop **aborts the mission** with a typed error (see §5.2) — this is
non-negotiable: silent iteration exhaustion is the exact bug we are
replacing.

Scope rules:

- Variables bound inside `body` are **not** visible outside the loop.
- Variables bound inside `verify` are also not visible outside.
- Loops expose exactly one synthetic binding outside them: `<name>.result`
  which is the value of the verify call's `output` on the winning
  iteration. If no name is given, the loop is anonymous and exports
  nothing.
- **v1 conservative reading**: outer mission bindings are *not*
  visible inside `body` / `verify` either. The loop block is
  hermetic. RFC §3.1 of the Draft revision did not spell out the
  reverse direction, so v1 takes the tighter interpretation; a
  follow-up RFC may relax this if a mission needs it.

### 3.2 Descoped forms

`chat` and `handoff` block forms proposed in the Draft revision are
**not part of this RFC**. See §10.

## 4. Semantics — precise

### 4.1 Termination (load-bearing)

Every `loop` block **must terminate deterministically without
inspecting runtime state external to the mission**. The static upper
bound on cross-agent calls is:

```
max_iters * (calls_in_body + calls_in_verify)
```

This bound is computed at compile time and enforced by the planner
refusing to emit an IR whose worst-case call count exceeds a global
cap (`IrConstraints.max_calls`, default 256, configurable per
mission).

### 4.2 Recursion depth

Existing `MAX_AGENT_DEPTH` (dispatch.rs:432) applies unchanged. A
`loop.body` that calls another agent still counts against the depth;
nesting a `loop` inside another loop is **rejected at compile time**
in v1 — the planner refuses any mission whose static loop nesting
exceeds depth 1. The reason: each nested loop multiplies the static
call bound, and v1's scope (the auto-review-loop pattern) does not
need nesting. Future relaxation is an RFC-edit with a gallery
consumer.

### 4.3 Mission runtime single-path invariant

**The `loop` block does not bypass `run_mission_inproc`.** Each
iteration of a `loop` goes through the same dispatch path that a
flat `let x = agent.ability(...)` goes through. The `services/`
layer (see §7) may cache warm sessions across iterations, but it
must never skip the mission context invariant check, the depth
guard, or the run-dir write.

### 4.4 `verify`'s contract with `done`

The last call in a `verify` block must produce output whose JSON
decoding contains a top-level boolean `done`. The loop terminates iff
`done == true`. Any other shape — missing field, non-boolean, absent
output — is a **hard abort** (§5.2). This forces verify authors to
make the terminate predicate explicit and inspectable; lossy implicit
"returned non-empty → good" rules are rejected at the IR level.

## 5. Errors

### 5.1 Compile-time rejects

- `max_iters` missing, ≤ 0, or > 32.
- `verify` block whose last statement is a `let` binding without a
  call, or whose final call targets a device (agent-only constraint).
- Static call-count bound exceeds `IrConstraints.max_calls`.
- Nested `loop` inside another `loop` (v1 — see §4.2).

### 5.2 Runtime errors (each aborts the mission)

- `LoopExhausted { name, max_iters }` — reached `max_iters` with
  `done == false`.
- `VerifyMalformed { iter, reason }` — verify output missing or
  non-boolean `done`.

All errors surface through the existing mission run dir as structured
JSON; no new error channel.

## 6. IR shape

Additive to `src/eal/ir.rs`. The `Call`-only `IrStep` flat struct
from Mission IR v1 becomes a tagged enum:

```rust
pub enum IrStep {
    Call(IrCall),                         // the pre-RFC variant
    Loop {
        name: Option<String>,
        max_iters: u32,
        body: Vec<IrStep>,
        verify: Vec<IrStep>,
        result_binding: Option<String>,   // <name>.result
    },
    // Chat / Handoff variants in Stage 1's IR scaffold are retained
    // for on-disk compatibility with missions that were compiled
    // under the Draft revision of this RFC, but the parser no
    // longer accepts those block forms and the executor's bail
    // points explicitly cite §10. See the PR-10 plan for removal
    // trigger.
}
```

`Loop` lowers to a sequential in-process executor (see §7); the
lowering is deterministic so that the "structural" equality used by
`scripts/trace-parity.sh` is preserved.

`MissionIr.phases` must handle nested steps. v1: `loop` is
**single-phase** (no parallel iterations). When any top-level item
is a `Loop`, the outer phase scheduler collapses to one-step-per-
phase in source order; pure-`Call` missions retain the pre-RFC
parallel-when-independent scheduling.

## 7. Daemon on / daemon off — behaviour matrix

Identical EAL source must produce identical user-visible results in
both modes; only the **observability surface** differs.

| Aspect | daemon online | daemon offline |
|--------|---------------|----------------|
| Loop iteration | scheduled by `services/loop_exec` | sequential in-process |
| Mid-iteration timeline streaming | yes (WS subscribers see each body step) | no (timeline materializes at end) |
| User can hot-interrupt from client | yes | no (SIGINT only) |
| `permit` flow (human-in-loop) | interactive via WS | fails closed — mission aborts |
| Persistence | `<agent-root>/runs/<id>/timeline.jsonl` | same |
| Final result semantics | identical | identical |

The daemon is a **performance and observability** overlay, not a
semantic one. Any test that runs only against daemon-online mode
must also pass in daemon-offline mode (modulo `permit` which is
defined to fail closed).

The daemon-online branch is currently hypothetical — `services/
loop_exec/` does not exist as a Rust module; see
`docs/open-questions/does-easynet-need-a-local-ws-control-plane.md`
for why. When a daemon consumer appears, this matrix's left column
describes the contract that implementation must satisfy.

## 8. Compatibility

- Mission IR v2 gains a tagged enum variant on what used to be a
  flat `IrStep`. Readers of `--emit-ir` must be updated; writers
  that only emit the existing `Call` variant remain valid.
- `scripts/trace-parity.sh` fixtures that use old flat IR will need
  a one-time refresh when a mission actually uses a `loop` block —
  this is the documented procedure when schema changes deliberately.
  Pure-`Call` missions are byte-identical.
- EAL parser: `loop`, `body`, `verify` become reserved keywords.
  Existing missions that use those as identifiers (none in the
  gallery today; grep-checked) will get a clear parse error
  pointing at the new grammar.

## 9. Acceptance — RFC is merged when

1. Ontology review signs off on this document (S13 item #1).
2. The `loop` example in §3 compiles against the prototype parser
   (PR-10 Stage 2).
3. `auto-review-loop.eal` rewritten using `loop { body, verify }` is
   semantically equivalent to the 4-round unrolled form — verified
   by hand-comparison of call sequence.
4. Worst-case call-count bound §4.1 rejects a pathological input at
   compile time (PR-10 Stage 3 test).

## 10. Audit: why `chat` and `handoff` were removed

The Draft revision of this RFC proposed three block forms. An audit
against the plan-audit clause (every scheduled entry must point at a
concrete consumer: a merged gallery mission, a tracker issue, a
named ontology constraint, or a failed-mission trace) eliminated
two of them:

**`handoff { }` — deleted.** No protocol ground, no gallery
consumer. The proposed semantics — "pack source agent's context,
invoke target agent with it" — is expressible in two flat EAL
statements (`let ctx = source.summarize(...)` then `target.chat(prompt: ctx.output)`) against existing v1 grammar. An
explicit block buys nothing.

**`chat { participants, max_turns, … }` — descoped to
open-question.** Two issues:

1. *Naming conflict*. `chat` is already an AXIOM agent *ability*
   name — `agent.chat(prompt: ...)` is the canonical single-agent
   invocation across the system. Re-using `chat` as an EAL
   block keyword for a *multi-agent conference* form introduces
   the same token at two semantic layers with two different
   meanings.
2. *No consumer at the block layer*. `easynet discuss` already
   exists as a CLI verb (`src/facade/cli/discuss.rs`,
   `easynet discuss --agents … --rounds … --topic …`) and
   orchestrates multi-agent conferences at the command layer.
   No gallery mission requests conferences *as an EAL block*.
   The block form was speculative.

The question "should EAL grow a `discuss { }` block that mirrors
the `easynet discuss` CLI verb at the language level?" is tracked
in `docs/open-questions/discuss-eal-block.md`. Trigger: a gallery
mission appears that genuinely needs the block form (e.g. a
conference nested inside a `loop`'s verify block, or a conference
whose transcript is consumed downstream by another EAL step).
Until then, operators compose via the CLI verb.

## 11. Open questions (tracked, not blocking `loop`)

- **Q1 — `while` without `verify`**: does a bare `while cond { body }`
  add enough over `loop { body verify }` to justify the grammar? Not
  in v1. Revisit after a year of `loop` usage.
- **Q2 — nested loops**: RFC §4.2 rejects nested loops at compile
  time in v1. A future RFC may relax this when a mission needs it;
  the static call-count bound enforcement remains.
- **Q3 — outer bindings visible inside `body` / `verify`**: v1 is
  conservative (§3.1). Relax when a mission needs it.

## 12. Implementation note

PR-10 implements this RFC in three stages, as landed:
- Stage 1 — IR node (`IrStep::Loop` enum variant).
- Stage 2 — parser (accepts `loop { body { } verify { } }` grammar).
- Stage 3 — executor (in-process sequential loop runner).

The RFC itself is frozen at merge; any deviation in PR-10 requires
an RFC-edit PR, not an implementation-only PR. This keeps the
decision surface inspectable independently of code diffs.
