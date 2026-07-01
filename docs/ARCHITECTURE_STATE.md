# EasyNet-Cli Architecture State

**Status:** stable basin (v0.2.0-typed-dispatch). Locked.
**Audience:** future contributors and future-self.
**Purpose:** orient quickly. Not a tutorial. Not a design rationale.
For the *why*, read the documents this file references.

This document records what the system **is**, not what it could be.
Everything below this line is invariant under the current architecture
boundary. Changes to any of it require an alignment session, not a
patch.

---

## 2026-06-21 supersession: Ability control plane

This file records the historical `v0.2.0-typed-dispatch` L2 execution basin.
Its statements about `AbilityName`, call paths, member-call syntax, and
`IrTarget` remain useful for that layer.

They are not the current authority model for daemon/plugin/device abilities.
For the current model, use `docs/design/ability-control-plane-model.md`.

In particular, older phrases such as "Ability is a method", "there is no
global ability identity", and "agent public abilities are methods" must not be
used to override the current split:

```text
AbilityDescriptor = versioned governed interface
AuthorityBinding = governance predicate over advertise + invoke
AbilityImpl = versioned executable binding
Daemon = projection + dispatch runtime
Receipt = versioned, verifiable execution fact
```

Read the "Ability semantics" section below as scoped to the historical L2
EAL typed-dispatch surface, not as a complete daemon control-plane ontology.

## Execution layer

**Mission runtime is the single execution path.** Every cross-agent
interaction in the system flows through `cli/mission_runs.rs::run_mission_inproc`.
There is no second path. The CLI verb `easynet agent send <name> "..."`
is sugar that constructs a single-line External EAL mission and hands
it to the runtime. The MCP `send_to_agent` tool is the wire-level form
of `<agent>.chat(<prompt>)` and reaches the runtime through the same
entry point.

Recursion is bounded by the typed `runtime::context::DispatchContext`
depth (max 2) and gated by a mission id whose run directory must exist.
`MissionContextGuard` installs that context in thread-local state and
restores the previous value on drop. `EASYNET_MISSION_ID` and
`EASYNET_AGENT_DEPTH` are reserved for subprocess entry only: the parent
runtime writes them into a child command's env map at the spawn boundary,
and fresh agent CLI processes reconstruct the typed context from them.
Legacy callers (`cli/think.rs`, `cli/discuss.rs`) wrap themselves in
`LegacyMissionContext` to satisfy the gate without going through the
full mission runtime; they are deprecated aliases per ontology §8.

## Identity layer

**`AgentId` is the only logical identity for agents.** It is a
struct of `{ tenant: String, name: String }`. Surface forms `claude`
(shorthand) and `silan/claude` (full) parse to equal values when the
tenant is `default`. `Display` always emits the canonical full form.
Equality is on the resolved pair, not on the input string.

Character class: `[a-z0-9_-]+`, ≤63 chars per segment, ASCII
lowercase only. Reserved-for-v2 forms (`a/b/c`, `claude#42`,
`easynet://...`) are explicitly rejected with named errors so future
v2 enablement only has to flip the rejection.

`AbilityName` is a typed string for the method-name slot of the call
path. It is **not** a peer of `AgentId` and **never** an identity. Its
character class allows `.` for the existing dotted-namespace
convention (`photo.capture`). The asymmetry between `AgentId` and
`AbilityName` is load-bearing — see `AGENT_IDENTITY.md` §10 for the
three hard prohibitions that protect it.

## Dispatch model

**Dispatch is purely type-driven.** The IR target is an enum:

```rust
pub enum IrTarget {
    Agent(AgentId),
    Device { node_id: String },
}
```

The interpreter dispatcher matches on this enum. There is no
`is_agent` string lookup, no `find_provider_for_ability` capability
matching, no `if registry.contains(name)` shortcut. The variant is
chosen at parser time (member-call → `Agent`, traditional → `Device`),
baked into the IR by the planner, and consumed unchanged by the
runtime.

The minimum unit of execution is the **call path** —
`(AgentId, AbilityName)` for agent calls, `(node_id, AbilityName)`
for device calls. Both halves are typed. They travel together. Neither
half is addressable on its own.

## Ability semantics

**Ability is a method, not a resource.** It exists only in the
context of a call path. There is no global ability identity, no
ability registry, no provider matching. `claude.chat` and
`codex.chat` are different methods that share a name; they are not
two implementations of the same ability.

The ontological grounding is the OOP view: an agent is an object,
its public abilities are methods, and its private skills are
implementation resources. A skill is not a private ability and is not
network-addressable. The encapsulation invariant is non-negotiable —
no CLI command, no SDK call, and no EAL construct may reach across an
agent boundary into a private skill.

## EAL surface invariant

**Member-call form is the only way to invoke an agent.** Traditional
form is strictly device-only. No implicit agent fallback is allowed.

```text
claude.chat(prompt: "hi")             → IrTarget::Agent(AgentId)
call "chat" on "node-1" with {...}    → IrTarget::Device { node_id }
call "chat" on "claude" with {...}    → REJECTED with hard error
                                          pointing at member-call form
```

The rejection happens at `run_mission_inproc` time via
`find_implicit_agent_fallback`, before any disk artifact is created.
The error message names the colliding agent, suggests the exact
member-call form, and references `docs/AGENT_IDENTITY.md`. Three
named tests guard the invariant: `no_implicit_agent_fallback_*` in
`cli/mission_runs.rs::tests`.

## External narrative discipline (avoid overclaim)

Anyone writing about EasyNet externally — papers, blog posts, talks,
investor decks — MUST describe the following with the precise
phrasings below. Three specific overclaims are easy to produce by
accident and will be attacked on sight by a careful reviewer:

1. **"Invocation is a protocol-level primitive."** Partially true.
   AXIOM defines invocation as a signed seven-parameter object; the
   Axon Rust SDK ships signed-path symbols (`call_mcp_tool_signed`,
   `InvocationEnvelope`). The Cli and backend today call through the
   *unsigned* MCP path (`call_mcp_tool_with_timeout`), so at protocol
   wire level, invocations in this deployment are RPC-with-audit-trail,
   not signed first-class invocations. Correct phrasing: "**We elevate
   invocation to a protocol-level abstraction, partially realised in
   the current system.**" Do not say: "Invocation is a protocol-level
   primitive" without the qualifier.

2. **"Axon is the execution runtime."** Not quite. Axon is an
   orchestration + admission layer. The actual code that runs
   abilities lives in CLI processes on devices (and in MCP servers
   those CLIs spawn). Axon dispatches `ExecCommand` / `CallMCPTool`
   to the device; the device's Cli process executes and returns.
   Correct phrasing: "**Execution is delegated to CLI-hosted
   runtimes, with Axon coordinating invocation lifecycle.**" Do not
   say: "Axon is a unified execution runtime" — reviewers who read
   `interop/mod.rs` will notice the delegation immediately.

3. **"EasyNet includes a proof / reputation / store / ledger layer."**
   Not yet. Those are Tier-2 services named in the ontology as
   planned extensions; no shipped code implements them today. Correct
   phrasing: "**Higher-layer services (proof, reputation, store,
   arbiter, sandbox) are planned extensions enabled by the
   protocol.**" Do not list them as system components.

These corrections are small in wording, large in credibility. The
single novel theoretical contribution (AXIOM's necessity argument
that HTTP-family protocols cannot satisfy Q1–Q6 without a mandatory
signed-byte profile) is load-bearing; overclaiming around it damages
the claim that *is* true.

## Forbidden patterns

The following are explicitly forbidden by the current architecture
boundary. Each has a named anti-regression mechanism. Do not introduce
any of these without an alignment session that supersedes the
documents listed.

| Pattern | Why forbidden | Guarded by |
|---|---|---|
| `target_node_id: String` (or any string-typed dispatch target) | Stringly-typed system; collapses identity into string. | `IrTarget` enum + Step 2 type gate |
| `is_agent(name: &str)` runtime classification | Routing decision based on string lookup; inverts the type system. | `AgentAwareDispatcher` matches enum; field deleted in Step 2 |
| `ability: String` on any IR-level type | Untyped method slot; invites string-format injection. | `AbilityName` newtype on `IrStep`, `StepTrace`, dispatcher trait |
| `AbilityRef { agent: AgentId, name: String }` as IR target | Service-registry model; collapses OOP into provider matching. | `AGENT_IDENTITY.md` §10 prohibition 3, prose invariants in `parser.rs` and `ir.rs` |
| `find_provider_for_ability(...)` capability matching in dispatcher | Inverts the call direction (ability → agent instead of agent.method). | `AgentAwareDispatcher` only routes by enum variant, never searches |
| Traditional `call ... on "<agent-name>"` silently routing to agent | Implicit agent fallback; pollutes the EAL surface semantics. | `find_implicit_agent_fallback` + `no_implicit_agent_fallback_*` tests |
| `easynet capability ...` CLI verb, MCP tool, or module | Premature ontology commitment per §7.3; would freeze a not-yet-resolved abstraction. | Ontology §7.3, vocabulary discipline in `AGENT_IDENTITY.md` |
| URI strings in `IrStep` or any L2 surface | Wrong layer; URA is L3 and not implemented. | `AGENT_IDENTITY.md` §2 Constraint 1 (string non-equivalence) |
| Adding fields to `AgentId` (e.g. `node_id`, `endpoint`, `public_key`) | Identity-vs-locator collapse; reintroduces the agent-as-degenerate-device error. | `AGENT_IDENTITY.md` §2 Constraint 2 + the comment on `AgentId` itself |

## What's deferred (do not start)

These are real future concerns. They are correct directions. They are
**not** the current PR. Picking any of them up requires the lockdown
to be re-opened by an alignment session.

- **L3 URA addressing layer.** `easynet:///r/...` URI scheme,
  canonicalization, signed envelopes, conformance vectors. See
  `../URA/README.md`. This is a protocol layer, not an EasyNet-Cli
  feature.
- **Marketplace/catalog layer.** `agent.ability` ranking,
  recommendation, discovery surface. This is a presentation layer
  parallel to L2, not part of it.
- **Paper-level system model.** Definition + invariants + diagram.
  Drawn from this document plus the ontology. Tracked as the next
  epic per ontology §10.
- **Tenant lifecycle, sharing, and authorization.** Currently
  `tenant` is just a name. The trust model is ontology §11.5
  recursion point.
- **Instance ids** (`<tenant>/<name>#<instance>`). Reserved for v2
  per ontology §6.4. Parser already rejects them with a named error.
- **`AbilityName` versioning.** `AbilityName` is currently a string
  newtype; future may become `Ability { name, version }`. The IR
  field name was deliberately chosen as `ability` (not
  `ability_name`) to leave room.
- **MCP mission handler unification.** Closed: MCP-facing mission
  handlers now delegate through `run_mission_inproc`. A second
  production mission execution path is no longer an acknowledged
  exception; it is a release blocker.

## Pointers

| Concern | Source of truth |
|---|---|
| Ontology (axiom layer) | `docs/easynet_ontology.tex` |
| Identity layer (this layer) | `docs/AGENT_IDENTITY.md` |
| Future protocol layer | `../URA/README.md` |
| Implementation: identity types | `src/shared/agent_id.rs` |
| Implementation: IR enum | `src/eal/ir.rs` |
| Implementation: dispatcher | `src/eal/interpreter.rs` |
| Implementation: single mission entry | `src/cli/mission_runs.rs` |
| Implementation: surface form invariant | `src/eal/parser.rs` (header comment) |
| Anti-regression tests | `src/cli/mission_runs.rs::tests::no_implicit_agent_fallback_*`, `src/eal/interpreter.rs::tests::*_lowers_to_*_target` |

## Test counts at boundary

```
unit + integration:  105 passing
ignored (require auth): 2
total:               107
```

End-to-end smoke green: `easynet agent send claude "..."` runs the
full chain (CLI → desugar → mission → analyzer → planner →
interpreter → AgentAwareDispatcher → claude binary → reply) and
displays the canonical agent form `default/claude` in the per-step
banner.

---

**This file is the architecture boundary.** Everything above is
locked. Read this before changing anything in `src/eal/`,
`src/cli/mission_runs.rs`, `src/agent/dispatch.rs`, or
`src/shared/agent_id.rs`.
