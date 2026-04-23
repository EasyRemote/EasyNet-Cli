# Open Question — Migrate CLI dispatch to a first-class Axon Invocation

**Status:** Open · **Trigger-based revisit** · **Owner:** Silan Hu · **Date:** 2026-04-23

## The question

AXIOM §5 (`document/concepts/AXIOM.tex`) specifies that every agent communication event is expressible as the seven-parameter `invoke(caller, callee, ability, subject, nonce, causal_context, args) → receipt`. An invocation is a *first-class* protocol entity only when all of I1–I5 (in-process) and P1–P6 (across-process) hold, AND the caller/callee signatures over canonical envelope bytes are emitted.

Today the CLI's `src/runtime/dispatch.rs::send_to_agent_with_depth` is "RPC with audit trail," not a first-class Invocation. It records timing, model, prompt, and response to `runs/<ts>/`, and (after PR-7) emits an event log through `PersistentLog` that honours P1–P6 — but it does not:

- Construct a signed `InvocationEnvelope` (caller / callee / subject / nonce / causal_context covered by Ed25519)
- Emit a callee-signed terminal `Receipt` with `prev_receipt_hash` chained to the caller's causal_context
- Carry an `ability_snapshot.content_hash` covering the skill manifest

The AXIOM §7 warning is explicit: a runtime that only reaches the RPC-with-audit-trail level will see the Mission / Task / background-task concepts silently return, because the runtime concept is too thin to carry their use cases. This open question tracks when the CLI should migrate.

## Why this is deferred

Three Axon-side artefacts are still marked `\deferred` in AXIOM:

1. **URA v2 namespace** for CLI-hosted agents. AXIOM §5.4 shows an example form `easynet:///r/org/<tenant>/<agent>` but no normative profile document pins it. An `AgentIdentity { uri, profile }` composite cannot be constructed without this.
2. **`document/profiles/DEFAULT_PROFILE.md`**. Profile selector (`easynet-strict-v2` vs `web-safe-v2`) is structurally bound to identity per AXIOM §5 Axis B; until the profile document exists, the composite's `profile` field has no canonical value.
3. **Discovery agent reserved URA**. A first-class Invocation from the CLI would typically cite the discovery-agent publish receipt in its `causal_context` (capability grant). Without the discovery agent, the first-hop `causal_context` has no typed predecessor.

Moving the CLI to signed envelopes before any of these three stabilises would either (a) hardcode a URA shape Axon later rejects, or (b) ship a profile string that does not match Axon's eventual canonical form. Both are one-way easy to get wrong.

## What PR-7 does instead

PR-7 adopts `easynet-axon` `PersistentLog` for the event log (P1–P6 compliance by inheritance, cross-SDK observable identity via `invocation_id`) but does **not** adopt `LocalRuntime::invoke_async` or `sign_invocation` / `sign_receipt`. Concretely:

- CLI allocates its own `invocation_id` (UUID-v4, scoped to this host). The id is recorded in `runs/<ts>/meta.json` and used as the `PersistentLog` file stem.
- Events emitted to the log follow the flat `InvocationEvent` shape (sequence, timestamp, type, payload) but carry no signatures.
- `causal_context` is not emitted. Nested CLI invocations (agent A sends to agent B within a mission) carry the parent's `invocation_id` as a free-form `parent_invocation_id` payload field, mirroring I4 parent/child semantics without the signed chain.

This is honest about what the CLI is today: an audit-grade RPC runtime, not a first-class Axon Invocation runtime. Every event the PR-7 log emits is consistent with I1/I3/I5 on-disk (no state change after terminal, FIFO within an invocation, monotonic sequence), so a future PR-9 migration can replay the log through the full envelope-signing path without a schema break at the event-shape layer.

## What would move this to a plan item

All three must hold:

1. AXIOM's `document/profiles/DEFAULT_PROFILE.md` exists in a normative (non-draft) state, pinning the URA namespace and profile selector for CLI-hosted agents.
2. AXIOM's discovery agent has a reserved URA and a stable `publish` ability signature (tracked separately in `docs/open-questions/retire-a2a-agents-json-label.md` because the same triggers apply there).
3. A concrete consumer surfaces that needs CLI-emitted receipts on the audit chain. Candidates: EasyNet-Proof replay of CLI-side execution, a compliance auditor running `verify_receipt_chain` across CLI-originated invocations, or an Alive product requirement that cross-agent CLI invocations carry non-repudiable signatures.

Without any one of the three, this stays open. Building signed envelopes against unstable URA / profile / discovery shapes would commit the CLI to decisions the upstream hasn't made yet.

## What PR-9 (or whatever it's called at the time) would do

Sketch — not a spec. Revisit when triggers fire:

- Construct `InvocationEnvelope` from `send_to_agent_with_depth` inputs, including caller/callee/subject URAs per the normative profile.
- Sign via `sign_invocation` (caller side) before admission.
- Emit the envelope to `LocalRuntime::invoke_async` instead of the current `adapter.invoke` call.
- Receive terminal `Receipt` with callee signature; write to `PersistentLog` as the terminal event.
- Chain mission steps via `CausalContext::Scalar(prior_receipt_ref)`.
- Optionally pin ability snapshots via `CausalContext::Scalar` on a pin-receipt (AXIOM §6 reproducibility).

Estimated scope when unblocked: 6–8 sessions. Not pre-reserved in the plan; the task list opens it when the triggers fire.

## Side issue discovered + partially addressed 2026-04-23: skill hash is not Q6

**Status:** Semantic drift acknowledged. Rust field renamed to `skill_tree_hash`. Q6 compliance still waits on signed-envelope work (see main body above).



`src/facade/cli/skill.rs::hash_tree` computes the skill's `content_hash` as SHA-256 over the sorted file tree of the skill directory (code only). AXIOM §6.1 (Q6, added to AXIOM on the `rev10-signed-mcp-wip` branch) explicitly rules this out:

> "A receipt whose snapshot is SHA-256 of code alone fails Q6, because the executed behaviour depends on schema and dependencies as well as code."

Q6's `ability_snapshot.content_hash` must cover (a) skill implementation bytes, (b) the invoked ability's public input/output schema, and (c) external dependency references resolved at execution time. The CLI currently covers only (a).

**Not a regression** — the field was introduced before Q6 was written. **Not a product bug today** — the CLI's `content_hash` is still a deterministic, reproducible identifier of the skill's code (proven by the `hash_tree_is_deterministic_across_platforms` unit test), useful for local integrity checks and `skill upgrade` diffs. It is just not the Q6 attestation.

**Why not fix now:** Q6 defers manifest canonicalisation to a profile document (AXIOM §6.1 names RFC 002). Expanding the hash to (b)+(c) before that canonicalisation rule exists means either (i) picking a byte layout the profile later rejects, or (ii) shipping a hash that is Q6-shaped but not Q6-interoperable. Same failure mode as signing without frozen envelope bytes.

**Resolved in part 2026-04-23:** Rust field is now `skill_tree_hash`. Commits:

- Cli `dd4b55b refactor(skill): rename InstallRecord.content_hash → skill_tree_hash` — Rust rename + serde-rename pin tests.
- EasyNet `c21c927 docs(skill): correct content_hash semantics in backend + Frontend` — backend Go comment and Frontend TS doc corrected.

**Still deferred:** the actual Q6 `ability_snapshot.content_hash`. That field lives on a signed terminal receipt (callee-side, post-hoc), not inside `InstalledSkill`, so a rename alone can't produce it. Waits on RFC 002 canonical manifest bytes + signed envelopes per the main body above.

## Why this is not rolled into PR-7's open questions

`docs/open-questions/axon-invocation-receipt-link.md` asks a narrower question: should CLI timeline events carry the `invocation_id` from an Axon `Receipt`? That question presumes CLI dispatch has already moved to first-class Invocation (so a receipt exists to link to). This document tracks the prerequisite migration. When PR-9 lands, the receipt-link question resolves by construction — the CLI timeline is the Axon log.

## Log

| Date       | Event                                                                                       |
|------------|---------------------------------------------------------------------------------------------|
| 2026-04-23 | Opened as part of PR-7 scope delimitation (α: PersistentLog adoption without signed envelope). |
| 2026-04-23 | Added "Side issue" section: skill `content_hash` covers code only (a), not Q6's (a)+(b)+(c). Discovered during cross-repo audit — AXIOM Q5→Q6 was added on `rev10-signed-mcp-wip`. |
| 2026-04-23 | Part-resolved: Rust field renamed `skill_tree_hash`; wire name unchanged (`content_hash`); backend + Frontend docs corrected. Q6 compliance still deferred. |
| —          | Revisit: **trigger-based**, when conditions 1+2+3 above all hold.                           |
