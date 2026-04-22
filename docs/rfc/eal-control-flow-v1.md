# RFC: EAL Control Flow v1 — `loop` / `chat` / `handoff`

| Field | Value |
|-------|-------|
| Status | Draft — awaiting ontology alignment |
| Scope  | EAL language + Mission IR v2 (additive) |
| Motivates | `gallery/case01-aris/missions/auto-review-loop.eal` (4-round unrolled) |
| Closes (partially) | Ontology S13 deferred item #1 "EAL loops/conditionals" |

## 1. Why this RFC

Three recurring multi-agent patterns are today expressed by either
(a) unrolling the loop by hand (see `auto-review-loop.eal` — same three
calls repeated 4 times), or (b) pushing the control flow outside EAL
into a shell script or CLI wrapper. Both paths lose the machine-auditable
structure that is the entire point of having an IR.

`loop { }`, `chat { }`, and `handoff { }` are the minimum three block
forms that let the unrolled patterns become declarative. Anything
smaller (e.g. a lone `while`) either can't express the verify semantics
or re-derives them at runtime.

The three forms are chosen so that **every EAL surface construct still
lowers to a deterministic Mission IR v2 whose shape is auditable before
execution** (ontology §5 "mission as proof-carrying plan").

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
  point. Services-layer (`services/{chat,loop_exec,schedule}/`) are
  optimizations — see §7.

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

  chat "triangulate" participants: ["claude", "codex", "llama"] max_turns: 3 {
    topic: "What's wrong with the proposed plan?"
    visibility: fan_out     // or round_robin
  }

  handoff {
    from: claude
    to: codex
    context_mode: summary   // full | summary | none
    prompt: "Finish the implementation per the plan above."
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

### 3.2 `chat` block

Required attributes:
- `participants: [agent_ref, ...]` — 2 to 8 agents. Each element is a
  registered agent name resolvable via the registry at compile time.
- `max_turns: <N>` — compile-time integer ≥ 1, ≤ 16.

Optional attributes:
- `topic: <string>` — seeded as the first user message.
- `visibility: fan_out | round_robin` — how each turn's output fans
  back to the other participants. Default `fan_out`.

Chat is **bounded** — there is no "run until quiet" variant. The
ontology requires every mission to terminate in a provable finite
number of cross-agent calls.

Exports one binding: `<name>.transcript` — a JSON array of
`{agent, turn, text}` objects.

### 3.3 `handoff` block

Required attributes:
- `from: agent_ref` — agent whose session/context is being packed.
- `to: agent_ref` — receiving agent.
- `context_mode: full | summary | none` — see §4.6.

Optional: `prompt: <string>` prepended to the handoff message.

Handoff is **sugar** for a two-step mission: emit the source agent's
context (mode-gated) as a string, then invoke `to.chat` with that
string as `prompt`. It produces one binding: `<name>.result` — the
receiving agent's reply.

## 4. Semantics — precise

### 4.1 Termination (load-bearing)

Every new block **must terminate deterministically without inspecting
runtime state external to the mission**. The bound is:

| Block    | Upper bound on cross-agent calls |
|----------|-----------------------------------|
| `loop`   | `max_iters * (calls_in_body + calls_in_verify)` |
| `chat`   | `max_turns * len(participants)` |
| `handoff`| 1 |

These bounds are computed at compile time and enforced by the planner
refusing to emit an IR whose worst-case call count exceeds a global
cap (`IrConstraints.max_calls`, default 256, configurable per mission).

### 4.2 Recursion depth

Existing `MAX_AGENT_DEPTH` (dispatch.rs:432) applies unchanged. A
`loop.body` that calls another agent still counts against the depth;
nesting a `loop` inside another loop does not reset depth. The planner
rejects any mission whose static call graph exceeds depth 2.

### 4.3 Mission runtime single-path invariant

**None of the three blocks bypass `run_mission_inproc`.** Each
iteration of a `loop`, each turn of a `chat`, and the two steps of a
`handoff` all go through the same dispatch path that a flat `let x =
agent.ability(...)` goes through. The `services/` layer (see §7) may
cache warm sessions across iterations, but it must never skip the
mission context invariant check, the depth guard, or the run-dir write.

### 4.4 `verify`'s contract with `done`

The last call in a `verify` block must produce output whose JSON
decoding contains a top-level boolean `done`. The loop terminates iff
`done == true`. Any other shape — missing field, non-boolean, absent
output — is a **hard abort** (§5.2). This forces verify authors to
make the terminate predicate explicit and inspectable; lossy implicit
"returned non-empty → good" rules are rejected at the IR level.

### 4.5 `chat` fan-out ordering and visibility

Participants receive turns in declared order on iteration 1; subsequent
iterations use `visibility` to decide what each participant sees:

- `fan_out`: Every participant sees every other participant's prior
  turns (transcript broadcast). Each agent answers independently in
  the same iteration; all replies land before next turn.
- `round_robin`: Only the immediately preceding turn is visible.
  Cheaper context, but participants cannot see each other except
  through the chain.

Ordering within an iteration is **declared order**, not
first-to-respond. This is to keep the IR replay deterministic.

### 4.6 `handoff` `context_mode`

- `full`: the last `runs/<ts>/response.md` of the source agent is
  included verbatim.
- `summary`: a bounded-length (4 KiB) summary of the source's last
  session. Generated by the source agent itself via an
  implicit `from.summarize(of: session)` call — that ability must
  exist on the source. If missing, the planner refuses to compile.
- `none`: only the static `prompt:` attribute is passed.

Mode `summary` is the common path for multi-agent pipelines where
the next agent doesn't need full context but does need the decisions.

## 5. Errors

### 5.1 Compile-time rejects

- `max_iters` missing, ≤ 0, or > 32.
- `max_turns` missing, ≤ 0, or > 16.
- `participants` < 2 or > 8 or contains an unregistered agent.
- `handoff.context_mode = summary` where `from` has no `summarize`
  ability.
- `verify` block whose last statement is a `let` binding without a
  call, or whose final call targets a device (agent-only constraint).
- Static call-count bound exceeds `IrConstraints.max_calls`.

### 5.2 Runtime errors (each aborts the mission)

- `LoopExhausted { name, max_iters }` — reached `max_iters` with
  `done == false`.
- `VerifyMalformed { iter, reason }` — verify output missing or
  non-boolean `done`.
- `ChatParticipantUnavailable { agent }`.
- `HandoffContextMissing { from }`.

All errors surface through the existing mission run dir as structured
JSON; no new error channel.

## 6. IR shape

Additive to `src/eal/ir.rs`. New variant on a currently-flat `IrStep`:
lift steps into a container.

```rust
pub enum IrStep {
    Call(IrCall),                         // today's IrStep
    Loop {
        name: Option<String>,
        max_iters: u32,
        body: Vec<IrStep>,
        verify: Vec<IrStep>,
        result_binding: Option<String>,   // <name>.result
    },
    Chat {
        name: Option<String>,
        participants: Vec<AgentId>,
        max_turns: u32,
        topic: Option<String>,
        visibility: ChatVisibility,
        transcript_binding: Option<String>,
    },
    Handoff {
        from: AgentId,
        to: AgentId,
        context_mode: HandoffContextMode,
        prompt: Option<String>,
        result_binding: Option<String>,
    },
}
```

All three new variants lower to one or more `IrCall`s at interpret
time; the lowering is deterministic so that the "structural" equality
used by `scripts/trace-parity.sh` is preserved.

`MissionIr.phases` must handle nested steps. v1: new blocks are
**single-phase** (no parallel iterations, no parallel chat turns).
Parallelism at the fan-out layer is a follow-up.

## 7. Daemon on / daemon off — behavior matrix (explicit)

Identical EAL source must produce identical user-visible results in
both modes; only the **observability surface** differs.

| Aspect | daemon online | daemon offline |
|--------|---------------|----------------|
| Loop iteration | scheduled by `services/loop_exec` | sequential in-process |
| Chat turn fan-out | `services/chat` broadcasts to WS subscribers | sequential in-process |
| Mid-iteration timeline streaming | yes (WS subscribers see each body step) | no (timeline materializes at end) |
| User can hot-interrupt from client | yes | no (SIGINT only) |
| `permit` flow (human-in-loop) | interactive via WS | fails closed — mission aborts |
| Persistence | `<agent-root>/runs/<id>/timeline.jsonl` | same |
| Final result semantics | identical | identical |

The daemon is a **performance and observability** overlay, not a
semantic one. Any test that runs only against daemon-online mode
must also pass in daemon-offline mode (modulo `permit` which is
defined to fail closed).

## 8. Compatibility

- Mission IR v2 gains a tagged enum variant on what used to be a flat
  `IrStep`. Readers of `--emit-ir` must be updated; writers that only
  emit the existing `Call` variant remain valid.
- `scripts/trace-parity.sh` fixtures that use old flat IR will need a
  one-time refresh — this is the documented procedure when schema
  changes deliberately.
- EAL parser: `loop`, `chat`, `handoff` become reserved keywords.
  Existing missions that use those as identifiers (none in the gallery
  today; grep-checked) will get a clear parse error pointing at the
  new grammar.

## 9. Acceptance — RFC is merged when

1. Ontology review signs off on this document verbatim (S13 item #1).
2. All four examples in §3 compile against a prototype parser (the
   prototype lives in PR-10 and is not part of this RFC).
3. `auto-review-loop.eal` rewritten using `loop { body, verify }` is
   semantically equivalent to the 4-round unrolled form — verified
   by hand-comparison of call sequence.
4. Worst-case call-count bound §4.1 rejects a pathological input at
   compile time in a prototype test (again, lives in PR-10).

## 10. Open questions (tracked, not blocking)

- **Q1 — `while` without `verify`**: does a bare `while cond { body }`
  add enough over `loop { body verify }` to justify the grammar? Not
  in v1. Revisit after a year of `loop` usage.
- **Q2 — nested loops** beyond depth 2: should the compile-time call
  bound be per-mission or per-block? Current draft: per-mission sum,
  same number.
- **Q3 — chat `visibility: adaptive`**: an agent decides who sees
  its turn. Tempting but introduces runtime-dependent transcript
  shape. Deferred until a real use case appears.

## 11. Implementation note (for PR-10)

PR-10 implements this RFC. The RFC itself is frozen at merge; any
deviation in PR-10 requires an RFC-edit PR, not an implementation-only
PR. This keeps the decision surface inspectable independently of code
diffs.
