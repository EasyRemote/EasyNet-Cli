# Spec — Agent Publish Mechanism

> **Status: SUPERSEDED** by `node-roster-label-v2.md` (2026-04-22, same PR-5a bundle).
>
> **Reason for supersession.** This spec conflated three independent layers:
> (1) node-level labels used as a discovery hint for Frontend rendering;
> (2) the SDK's `publish_capability` API, which handles signed, distributable
> capability *packages* (tar.gz artefacts), not agent abilities; and (3) the
> Axon protocol-level publish path, which AXIOM §6.2 specifies as an invoke
> against a Tier-2 *discovery agent* exposing `publish` / `unpublish` /
> `lookup` abilities. The "hybrid mechanism A + mechanism B" decision in
> this file mixed (1) and (2) and ignored (3). Neither (1) nor (2) is an
> Agent-layer publish; (3) is but is not yet implementable (AXIOM marks the
> discovery-agent URA and ability signatures as `\deferred` pending
> `document/profiles/DEFAULT_PROFILE.md`).
>
> **What replaces it.** `node-roster-label-v2.md` covers the v1→v2 format
> flip of `a2a.agents_json` strictly as a node-level roster hint for
> Frontend rendering. It explicitly does not call itself a publish
> mechanism. The retirement path for that label, once AXIOM §6.2 lands, is
> tracked in `../open-questions/retire-a2a-agents-json-label.md`.
>
> **Also superseded.** `publish-json-format.md` in this same directory —
> the `publish.json` state machine was built on top of this spec's wrong
> framing and has no consumer under the corrected scope.
>
> **Also retracted, inside the content below.** "Minimum Axon SDK version
> 1.2" — fabricated; the real SDK is `easynet-axon 0.55.2`. Any reader
> who has to fall back on this file should not act on that number.
>
> The original content follows unchanged so the line of reasoning remains
> auditable. Do not build on it.

---

## Problem

An agent registered on a developer's machine should be reachable from the EasyNet federation as `easynet://agents/<owner>/<name>` with one or more `<agent>.<verb>` abilities. PR-3 put the agent on disk (`agent.toml` + `abilities/*.ability.toml`); PR-4 made `agent publish --dry-run` print the ToolSpec that *would* be published. This spec chooses how the live publish in PR-5b actually wires each ability onto Axon so a peer can invoke it.

The CLI never runs a daemon in PR-4, so "reachable" means: while the CLI process is running (`easynet agent send` / `easynet daemon` / an interactive MCP session), the adjacent Axon node accepts incoming RPCs targeting `<agent>.<verb>` and dispatches them through this node's mission runtime.

## Two mechanisms the SDK offers

The Axon Rust SDK offers two distinct paths for making something callable:

### Mechanism A — `DendriteBridge::publish_capability` + `AbilityToolAdapter::register`

- `publish_capability` advertises a *capability package* (tar.gz, signed manifest, deployable). Hub replicates the package to any node the caller wants.
- `AbilityToolAdapter::register(name, handler, spec)` binds a live Rust closure as the dispatch handler for a registered ability name. Incoming RPCs against `<agent>.<verb>` land in the closure.
- Discovery surface: the Axon node advertises the ability through its standard capability table; peers find it via capability queries, not through a2a.

### Mechanism B — `a2a.agents_json` labels only

- No `publish_capability` call. We attach a JSON label (`a2a.agents_json`) to our Axon node that enumerates our local agents and their abilities.
- Peers discover the ability by reading node labels.
- Invocation is *not* direct. A peer sends `send_a2a_task { target: "<agent>.<verb>", payload }` to our node; our node's `a2a` adapter unpacks it and routes through our mission runtime.

## Comparison

| Concern                   | A (publish_capability)                 | B (a2a labels)                      |
|---------------------------|----------------------------------------|--------------------------------------|
| Axon tool visibility      | First-class tool on our node           | Hidden behind `send_a2a_task`        |
| Invocation call shape     | `invoke_ability <agent>.<verb>`        | `send_a2a_task { target, payload }`  |
| Discovery surface         | Capability table + `a2a_card`          | `a2a.agents_json` only               |
| Intrusiveness             | Medium — adapter register per ability  | Low — label update only              |
| Package semantics         | Treats local subprocess as a package (misfit) | None — label is honest about locality |
| Remote-peer UX            | Peer sees `<agent>.<verb>` like any other ability | Peer must know the a2a protocol     |
| Rollback blast radius     | Must unregister + unpublish            | Clear label, one write               |
| Minimum Axon SDK version  | 1.2+ (AbilityToolAdapter + publish)    | 1.0+ (labels only)                   |
| Future daemon path        | Trivial — re-register on daemon start  | Still route via a2a — no change      |
| Proof / audit             | Invocation receipt per call            | Receipt through a2a send path        |

## Decision

**Hybrid, leaning on A for dispatch and on B for discovery.**

- Dispatch: each `abilities/<verb>.ability.toml` becomes one `AbilityToolAdapter::register(...)` call with the manifest's `input_schema` as the ToolSpec schema and a closure that invokes `runtime::dispatch::send_external(<agent>, <prompt>)`. Names registered verbatim as `<agent>.<verb>` (guaranteed unique by the AgentSpec name validator — no dots allowed in agent names).
- Discovery: the same manifest list is also reflected into `a2a.agents_json[*].skills` (v2 schema — see `a2a-v2-schema.md`). This reuses the existing label-based discovery surface; the Frontend consumes the backend's normalized `/api/v1/agents` and is unaffected. The backend's `node_mapper.go::ParseAgentsJSON` is rewritten for v2 in a paired companion PR (see `a2a-v2-schema.md` §Files that change). There is no tolerant-parser coexistence window — both ends flip to v2 together.
- `publish_capability`: **not used** in PR-5b. Reason: a locally-installed AI agent is not a distributable package; the runtime is the operator's machine, not the tar-gz artifact. Treating it as one creates a replication pathway we never want exercised.

The name "publish" is kept for the CLI verb because that's the human-facing concept; what it *does* internally is "register locally + attach label for discovery."

### What the decision rejects

- **Pure B (labels only).** Peers would be forced to know `send_a2a_task` even for the single most common operation (chat). The Axon wire-level view would have our abilities hidden one level deeper than every other ability on the network. That's a discoverability tax we should not charge.
- **Pure A (publish_capability included).** Package semantics misfit; rollback becomes a two-step dance (unregister + unpublish) with a failure mode where one succeeds and the other doesn't. No upside since the operator's machine cannot replicate anyway.

## Cost

- AbilityToolAdapter closures need to outlive the Axon bridge; the bridge's reconnect (PR-A3 `ReconnectingBridge`) must re-register on every reconnect. This is a known Axon SDK pattern but adds ~40 lines to `publish/` for the closure-rebind dance.
- Dual write: every `agent publish` writes both the adapter registration *and* the a2a label. If one succeeds and the other fails, `publish.json` (see `publish-json-format.md`) lands in `partial` state and `agent doctor` flags it.

## Future migration path

If a later release wants pure A (drop the a2a labels) because the Frontend migrates to capability queries:

1. Keep writing both for at least one release window.
2. Flip the Frontend to query capabilities instead of parsing `a2a.agents_json`.
3. Emit a deprecation warning on any label read.
4. Stop writing labels one release later.

If a later release wants pure B (drop the adapter):

1. This is a feature regression for any existing consumer that does `invoke_ability <agent>.<verb>` directly. Not planned.

## Minimum Axon SDK version

**Axon SDK ≥ 1.2** (required for `AbilityToolAdapter::register` with per-ability JSON schema).

If Cargo resolves to a lower version, `agent publish` must refuse at runtime with a clear version mismatch error. PR-5b carries the version check; it is out of scope for this spec.

## Impact on PR-5a / PR-5b

- `a2a-v2-schema.md` fixes the label format both paths must agree on.
- `publish-json-format.md` captures the dual-write state transitions so `agent doctor` can diagnose mid-flight failures.
- PR-5b implements mechanism A dispatch + mechanism B discovery per this decision; it does *not* call `publish_capability`.

## Open question deferred

Whether each AbilityToolAdapter invocation should also emit an `invocation::Receipt` (for EasyNet-Proof replay) is tracked in `docs/open-questions/axon-invocation-receipt-link.md`. The decision there is out of PR-5b scope and will be revisited 30 days after PR-7 merges.
