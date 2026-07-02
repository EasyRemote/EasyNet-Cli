# Open Question — Retiring the `a2a.agents_json` Node Roster Label

**Status:** Open · **Trigger-based revisit** (not date-based) · **Owner:** Silan Hu · **Date:** 2026-04-22

## Why this label exists

The `a2a.agents_json` node label (spec: `../spec/node-roster-label-v2.md`) is a node-level discovery hint the CLI attaches to its Axon node registration. The EasyNet backend parses it to enumerate what agents each device currently hosts, so the Frontend's Agents page can show an operator-level list.

It is **not** an Agent-layer publish. Publishing an agent ability in the AXIOM §6.2 sense means invoking the Tier-2 *discovery agent*'s `publish` ability and receiving a signed receipt, and that path is not yet implementable — AXIOM marks the discovery agent's reserved URA and ability signatures as `\deferred`, pending `document/profiles/DEFAULT_PROFILE.md` on the Axon side.

The label exists to do the right thing with what's available today: surface agents in the Frontend *now*, at a cost of one JSON string per node registration. It is technical debt measured in months, not years — but it is debt.

## The retirement condition

This label retires when **both** of these become true:

1. **Axon's discovery agent has a concrete, implementable contract.** Observable gate: `document/profiles/DEFAULT_PROFILE.md` exists in EasyNet-Axon's `document/profiles/` (or another path that pins the reserved URA + `publish`/`unpublish`/`lookup` ability signatures). A grep of the Axon repo root for `"DEFAULT_PROFILE"` and `"discovery agent"` should return a normative (non-draft) document.
2. **A way to read the discovery agent from the backend exists.** Observable gate: either a `ListA2aAgents` / `ListDiscoveredAgents` RPC on the bridge, or a typed helper in the Axon Go SDK that takes the Axon client and returns the agent list. Today `DendriteBridge::list_a2a_agents` exists on the Rust SDK but its wire contract pre-dates the AXIOM discovery agent and is unrelated.

If only one of the two is true, the label stays. Half-retiring (CLI writes receipts but backend still reads the label) produces the worst of both worlds — drift between what the federation thinks it published and what the Frontend shows.

## The retirement PR shape (when both triggers fire)

This is future work, not current work. Sketched here so a reader in the future has a starting point rather than re-deriving it.

1. **Axon side**: `document/profiles/DEFAULT_PROFILE.md` already pins the discovery agent; surface the appropriate RPC or typed helper.
2. **EasyNet-Cli**: replace the `register_node_with_options(..., labels: a2a.agents_json=...)` call with an invoke against the discovery agent's `publish` ability, one per registered agent. Persist the returned `invocation_id` / `receipt_hash` in a local ledger (not `publish.json` from the retracted spec — that was designed for the wrong thing; the ledger here is just the SDK's invocation log).
3. **EasyNet backend**: swap `ParseAgentsJSON` readers for the new RPC / helper. The on-wire label is left blank during a transition window, then removed from `register_node_with_options`.
4. **Golden fixture**: `tests/fixtures/a2a-v2/golden.json` retires with the label.

The retirement PR will not re-introduce a tolerant parser on the backend for a transition period — in the single-owner single-repo-pair topology, both sides land in the same release window. The transition window is "the gap between the two PRs merging," minutes.

## What to do in the meantime

- The label is live until retirement. Treat it as a Frontend-rendering contract, not as a publish contract.
- `agent publish --dry-run` (PR-4) is allowed to keep its name — the CLI verb is an operator-facing concept that will become honest once retirement lands. Until then its scope is "enumerate what the label would advertise"; it does not claim to perform a protocol-level publish. The spec for this CLI verb's current behaviour lives in `agent publish`'s implementation in `src/cli/agent.rs::run_publish`.
- Do not build a `publish/` Rust module, a `publish.json` local ledger, or a publish state machine against the current label. Those were designed in the retracted specs (`agent-publish-mechanism.md`, `publish-json-format.md`) under the wrong mental model.

## Log

| Date       | Event                                                            |
|------------|------------------------------------------------------------------|
| 2026-04-22 | Opened as part of PR-5a v3 cleanup. Retracts the "hybrid A+B publish mechanism" framing of the superseded specs in `../spec/`. |
| —          | Revisit: **trigger-based**, check when either condition 1 or condition 2 above flips. No calendar date. |
