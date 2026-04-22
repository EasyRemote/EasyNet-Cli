# Open Question — Linking CLI Mission Runs to Axon `invocation::Receipt`

**Status:** Open · **Revisit:** PR-7 merge + 30 days → fossilise as decision · **Owner:** Silan Hu · **Date:** 2026-04-22

## The question

When a peer invokes `<agent>.chat` on this CLI's Axon node, three artefacts exist:

1. **Axon invocation receipt** — the SDK's `invocation::Receipt`. Chain-of-custody record for EasyNet-Proof. Contains caller, target, input digest, output digest, timing.
2. **Mission run directory** — `<agent-root>/runs/<ts>/`. CLI-side artefact: meta.json + trace.jsonl + optional stdout/stderr capture. Today the authoritative *local* record of what the agent did.
3. **AgentSession timeline** (PR-7) — `<agent-root>/runs/<id>/timeline.jsonl`. Append-only event log of the agent's streamed output; replayable.

These three artefacts exist in three places (Axon, local FS, local FS) and describe overlapping slices of the same event. **Should they be linked?**

Two interpretations are reasonable and mutually exclusive:

- **Linked**: on every dispatch from a remote-origin invocation, the CLI writes the `invocation::Receipt.id` into `meta.json` and into every `timeline.jsonl` event. EasyNet-Proof's `proof.replay(invocation_id)` can then pull CLI-side run artefacts to reconstruct the full execution. Cost: CLI takes a hard dependency on SDK's invocation module; timeline writes must stay synchronous with receipt emission or the linkage breaks.
- **Unlinked**: Axon owns receipts; CLI owns run dirs + timelines; the two record the same event twice, with no cross-reference. Replay on the Axon side reconstructs *input → output*; replay on the CLI side reconstructs *what the agent actually did step-by-step*. No consumer exists today that needs to join them.

## Why this isn't decidable now

The decision turns on one thing:

**Does any consumer need the join?** Today, none. EasyNet-Proof's replay story is still at design stage. The Frontend's AgentDetailPage is unscheduled. Building the linkage before there's a consumer is the premature-abstraction trap the industrial discipline rejects.

A correction from an earlier draft of this document: it claimed SDK stability was a second open variable (`axon::invocation` module "still churning"). That characterisation was under-sourced. A walk of `EasyNet-Axon/sdk/rust/src/invocation/` shows `axiom.rs` with canonical-bytes encoders, `ReceiptBody` / `InvocationEnvelope` types, Ed25519 sign+verify helpers, and a parity constraint ("byte-identical output to Python/Go/Node/Java/Swift reference encoders for every conformance vector in `sdk/conformance/cases/axiom-*.yaml`"). The module is stable by any reasonable read. The real open variable is the one above — does anyone need the join — and nothing else.

## What we can commit to in PR-7 regardless

PR-7's timeline.jsonl writer will emit a structured event per turn and include a `InvocationOrigin` field that distinguishes `Local { interactive_permission }` from `Remote { caller_node, request_id }`. The `request_id` slot is present whether or not we fill it with the Axon receipt id.

**TODO anchor** (to be placed at `src/runtime/session.rs` in PR-7): a comment block naming this open question and listing the two future edits needed if we decide "linked":

```rust
// TODO(invocation-receipt-link): if decision is "linked",
//   1. pass receipt.id into `timeline::Event::new_remote`
//   2. stamp `meta.json.invocation_id = receipt.id` in the dispatch path
// See docs/open-questions/axon-invocation-receipt-link.md
```

The TODO is the only PR-7 cost of leaving this open.

## Options on the table

### Option A — Link eagerly (in PR-7)

Cost:
- CLI depends on `axon::invocation` module. The module is stable enough to consume (see correction above); this cost is the usual cost of depending on a crate, not an outsized churn risk.
- Adds ~50 lines to PR-7 (pass id through `send_external`, stamp in meta.json, include in every timeline event).
- Timeline write order must be: Axon receipt emitted → id captured → first timeline event written. Breaking that order means timeline events exist without an id.

Benefit:
- EasyNet-Proof replay of CLI-side execution becomes possible without a schema migration later.
- AgentDetailPage (Frontend) can link from "incoming call" to "full run trace" with one id.
- Less work total if we do eventually need the linkage.

### Option B — Don't link (document the non-decision)

Cost:
- If EasyNet-Proof replay grows a CLI-side need later, we do a migration: a PR that back-fills existing timelines with receipt ids where they can be reconstructed from logs, and stamps going forward. Migration is non-trivial because historical timelines may be missing the correlate.

Benefit:
- PR-7 stays scoped to Session + Timeline; no SDK dependency churn bleeds into it.
- If EasyNet-Proof's design stabilizes in a different direction (e.g. Proof reconstructs the CLI-side view from Axon-side data alone), we've avoided a dead-end coupling.

### Option C — Link lazily (emit id in meta.json only; not in timeline events)

Cost:
- Small (~10 lines in PR-7).
- `meta.json.invocation_id` provides the join key; timeline.jsonl events remain invocation-agnostic.

Benefit:
- Minimum viable linkage. A Proof-side consumer that wants "run dir for receipt X" can find it via `meta.json` scan (one file per run).
- Leaves door open for C→A upgrade in a later PR (add id to timeline events) without a schema break.

## Decision rule

**Revisit date:** PR-7 merge + 30 days.

At the revisit:

- If a concrete consumer (Proof replay, Frontend AgentDetailPage, or internal tooling) has surfaced a real need for the join → **go to A**, schedule a follow-up PR.
- If no consumer has surfaced → **go to C**, ship minimum viable linkage (stamp id in `meta.json` only, not in every timeline event). Future consumers can upgrade C→A without a schema break.
- If a revisit produces no new signal twice in a row → fossilise as B (permanent non-decision), documented as such.

## Blocker status for other PRs

- **PR-5b-relabel** (node roster label v1→v2 migration): not blocked. The label is a node-level hint, not an invocation; no receipt linkage needed.
- **PR-6** (daemon + WS): not blocked. Daemon hosts sessions; it does not itself emit receipts.
- **PR-7** (session + timeline): not blocked. TODO anchor + `InvocationOrigin` field carry the decision surface; the question is "do we fill Remote.request_id with receipt id or with an independent uuid" — either works for PR-7's own tests.

## Post-decision action (whichever way it goes)

Close this file: rename to `docs/spec/invocation-receipt-link-<A|B|C>.md`, write the final decision + rationale, and land a PR that:

- Removes the TODO anchor
- Either implements the linkage (A or C) or adds a one-line comment in `meta.json` schema saying "no invocation_id by design; see spec"
- Notes the outcome on the ontology side of EasyNet main repo if it affects EasyNet-Proof's replay shape

## Log

| Date       | Event                                               |
|------------|-----------------------------------------------------|
| 2026-04-22 | Opened as part of PR-5a cross-repo spec bundle      |
| —          | Revisit scheduled for PR-7 merge + 30d              |
