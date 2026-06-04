# EasyNet CLI — Full Command Review

**Date:** 2026-05-30 · **Reviewer:** PM pass · **CLI baseline:** `easynet` v1.33.4
**Layers reviewed:** EasyNet-Axon (protocol) · EasyNet-Cli (shipped) · EasyNet (product vision)

This document lists **every command the CLI should have**, each tagged with:

- **Status** — `SHIPPED` / `PARTIAL` / `MISSING`
- **Priority** — `P0` (the product can't be itself without it) / `P1` / `P2`
- **Backing** (see §-1 for why this split exists):
  - `proto-rpc` — one of the **6 surviving wire RPCs** (`Invocation`'s Invoke/InvokeStream/InvokeBidi + `PayloadTransfer`'s 4). These are the only true RPCs left.
  - `proto-ability` — an Axon capability that exists, but **as a restated Ability** (RFC-001 P1.2), invoked *through* `Invoke`. The cited proto file:line is where the request/response **message** still lives; the canonical name is the **ability name** (e.g. `identity.get_trust`), not a dead RPC.
  - `roadmap` — planned, no proto/ability yet.
  - `cli-only` — no network call.
  - `ontology` — defined in the ontology doc but **not yet a registered runtime ability** (spec row, no handler).

The review is structured so we can **walk it one section at a time**. Each command has a one-line
*what it's for* and, where relevant, the Axon primitive it maps to (with file:line).

---

## North Star (the bar we measure against)

> Every capability gets a **URA**; the capability never leaves its owner's machine; callers get
> **results + a receipt** for billing, liability, and audit. Load-bearing verbs: **discover, invoke,
> receipt, bill**.

The CLI today nails **invoke** and **receipt**. It is weak on cross-owner **discover** and missing
**trust** and **bill**. That gap is the spine of this review.

---

## SECTION -1 — Backing model: after RFC-001, almost everything is an Ability

> **Read this before trusting any "Backing" cell.** A multi-agent audit (2026-05-30) found that an
> earlier draft cited dead RPCs. This section is the correction — and it *strengthens* the doc's core
> thesis (everything is an ability), it doesn't weaken it.

**The fact (verified against `EasyNet-Axon/docs/rfc/AXON-RFC-001-restatement-mapping.md` + every proto's
`collapsed` comment):** of **150 RPCs across 13 proto files, 140 were DELETED and restated as
Abilities** invoked through one primitive. Only **two services survive on the wire:**

- **`Invocation`** (invoke.proto:88) — exactly 3 RPCs: `Invoke`, `InvokeStream`, `InvokeBidi`.
- **`PayloadTransfer`** (transfer.proto:25) — byte/stream substrate `Invoke` can't carry.

Every other service (`ControlPlane`, `Policy`, `Capability`, `Federation`, `Mission`, `Identity`,
`Observe`, `Admin`, `Voice`, `Stream`, `StateSync`) carries an in-tree
`// RFC-001 P1.2.x: <Service> service collapsed` comment. Their request/response **messages still
exist** (transitional), but the **RPCs are gone** — each is now an ability called via `Invoke`.

**Why this matters for the whole doc:** a former call like `GetNodeTrust` is not a dead end — RFC-001
restates it as the ability **`identity.get_trust`**. But a restated ability name is not enough by
itself: CLI implementation should treat each row as **message exists; callable ability must be
confirmed/published** unless the current Axon runtime has a registered handler. That is the difference
between cheap CLI ergonomics and cross-repo protocol/ability-publication work.

**RPC → ability map for the abilities this doc references** (from RFC-001 mapping §):

| Old RPC (dead) | Restated ability (canonical) | Used in |
|---|---|---|
| `GetNodeTrust` / `SetNodeTrust` | `identity.get_trust` / `identity.set_trust` | §0, §2, §4 |
| `WhoAmI` | `identity.whoami` | §0, §1 |
| `CreatePolicy`/`UpdatePolicy`/`GetPolicy`/`ListPolicies` | `policy.publish` / `policy.get` / `policy.list` | §4, §4.5 |
| `PolicySimulate` / `GetDecision` | `policy.simulate` / `policy.get_decision` | §4, §4.5 |
| `CreateOverride` / `RevokeOverride` | `policy.create_override` / `policy.revoke_override` | §4, §4.5 |
| `GrantConsent` / `RevokeConsent` / `ListConsents` | `consent.grant` / `consent.revoke` / `consent.list_grants` | §3, §4, §4.5 |
| `SignPackage` / `PromotePackage` / `RollbackCapability` | `capability.sign_package` / `capability.promote_package` / `meta.rollback` | §3 |
| `ResolveCapability` | `capability.resolve` | §3 |
| `InstallCapability` | `meta.acquire` (AAL §3.4) — same primitive as `learn`, §2.5 | §2.5, §3 |
| `ListFederatedNodes` / `JoinFederation` | `federation.resolve` / `federation.federate` | §3, §8 |
| `WatchMission`/`GetMissionTimeline`/`AbortMission`/`RetryStep`/`ListMissions` | `mission.watch` / `mission.get_timeline` / `mission.abort` / `mission.retry_step` / `mission.list` (all via `Invoke`/InvokeStream) | §6, §6.1 |
| `GetNetworkHealth`/`GetSLOStatus`/`GetBurnRate`/`WatchEvents` | `observe.network_health` / `observe.get_slo_status` / `observe.get_burn_rate` / `observe.watch_events` | §8 |

**Convention for the rest of this doc:** `Backing: proto-ability (identity.get_trust ← control.proto:177)`
means "ability `identity.get_trust`, whose transitional message lives at control.proto:177." Line
numbers below were re-pinned against current proto HEAD (the audit found the old draft was pinned to a
pre-RFC-001 revision — hence its ~off-by-3 line drift).

---

## The Seven Value Axes (product spine — read this first)

The Axon-domain sections below answer *"what does the protocol expose."* This matrix answers the
question that actually matters to a user: **"is it easy to ___?"** Every axis is a promise; each row
gives the promise, the **main command chain** that delivers it, its maturity, and the headline gap.
Sections §0–§9 each note which axis they serve.

| # | Axis | The promise (user words) | Main chain | Maturity | Headline gap |
|---|---|---|---|---|---|
| 1 | **easy to GET** | *See it, use it.* **Two routes, both owner-driven:** (A) default — call it where it lives; (B) optional — the owner *teaches* it to you. | A: `ability show` → `ability invoke` · B: owner `ability teach` → learner `ability learn`/`study` | 🟡 route A ✅ / route B 🔴 | Route A (remote invoke, capability never leaves owner) is shipped. Route B (teach/learn) is **ontology-only**: `meta.acquire`/`meta.forget` are *defined in the ontology* (ontology:220) but have **no runtime handler** — only `meta.describe` + `meta.list_abilities` are registered today. The safety gate (`InstallPolicy`, capability.proto:237) exists. So Route B is a deeper gap than "no CLI verb": there's no handler either. NOT `pull` — see §2.5. |
| 2 | **easy to USE** | Find and call a stranger's capability without reading docs. | `discover "<intent>"` → `invoke <ura>` | 🟠 invoke ✅ / discover ❌ | No cross-owner `discover` (§0/§3). `invoke` is solid. |
| 3 | **easy to MANAGE** | Operate my fleet — devices, agents, runtime, health. | `device …` / `agent …` / `runtime …` / `health` | 🟢 strong | Local mgmt ✅; network-level `health`/`slo`/`burn-rate` missing (§8). |
| 4 | **easy to ORGANIZE** | Make the whole network (agents/abilities/devices/missions) a *navigable structure*, not a flat list. | `discover --tree` + `family` projection (§3.6) | 🔴 missing | No family/tree projection. Flat lists throw away real structure (§3.6). |
| 5 | **easy to PROTECT** | Guard **service + data + resource**: who's trusted, who's allowed, isolation, E2E encryption, sandbox, quota. | `trust …` + `policy …`/`permission …` + sandbox/quota/E2E | 🔴 mostly missing | trust ❌, policy/permission ❌ (§4/§4.5). Encryption/quota exist in proto, unexposed. |
| 6 | **easy to ACCOUNT** | Tamper-evident ledger: *who called whom, did what* — audit, compliance, liability. (NOT money.) | `invocation list/show/trace` + `policy why` | 🟢 strong | Receipt/ledger surface is the CLI's best story. `policy why` (decision explain) missing. |
| 7 | **easy to ECONOMIC** | The money *on top of* the ledger: cost, balance, agreements, revenue-split. (NOT the ledger itself.) | `wallet …` + `agreement …` | 🔴 missing | Whole axis blocked on Axon protocol (no proto yet — §5). |

**How to read maturity:** 🟢 shipped & coherent · 🟡 partial (a step in the chain is missing) ·
🟠 split (core verb shipped, entry/discovery missing) · 🔴 missing or protocol-blocked.

**One line per axis:** GET (almost — fix `pull`) · USE (add `discover`) · MANAGE (good) ·
ORGANIZE (projection tree) · PROTECT (the big hole: trust+permission) · ACCOUNT (the crown jewel) ·
ECONOMIC (needs protocol first).

> **Account vs Economic are deliberately separate axes.** Account = the *truth* (immutable record of
> what happened). Economic = the *value* (money settled on that truth). You can ship Account without
> Economic (audit-grade today); you cannot ship Economic without Account (you bill *from* the ledger).

---

## SECTION 0 — Quickstart (highest-frequency verbs)

> **Serves axes: GET (publish/online) + USE (entry to discover/invoke).**

| Command | Status | Pri | Notes |
|---|---|---|---|
| `easynet join [TOKEN]` | SHIPPED | — | Shortcut for `device join`. Good. |
| `easynet start` | SHIPPED | — | Shortcut for `runtime start`. Good. |
| `easynet stop` | SHIPPED | — | Shortcut for `runtime stop`. Good. |
| `easynet whoami` | **MISSING** | P1 | Today buried in `auth whoami`. Promote to top-level — it's a daily verb. Backing: `WhoAmI` (identity.proto:214). |
| `easynet discover "<intent>"` | **MISSING** | **P0** | The marketplace moment. See §3. Backing: `ListFederatedNodes` (federation.proto:292). |
| `easynet invoke <ura>` | PARTIAL | P1 | Exists as `ability invoke`; consider top-level alias for the core verb. |

**Review note:** the three shipped shortcuts are the right idea. The argument here is that
`discover` / `whoami` / `invoke` are *also* daily verbs and deserve the same first-class spelling.

---

## SECTION 1 — Identity & Auth (`auth`)

| Command | Status | Pri | Notes |
|---|---|---|---|
| `auth login [--register-if-missing] [--hub] [--password] [--nickname]` | SHIPPED | — | Flag is `--hub` (auth.rs:128), not `--host`. Also accepts `--nickname` (auth.rs:140). |
| `auth logout` | SHIPPED | — | |
| `auth whoami` | SHIPPED | — | Keep, but also surface at top level (§0). |
| `auth pair [--quiet]` | SHIPPED | — | Mints device-pairing token; pipeable. |
| `auth devices` | SHIPPED | — | HTTP-API device list. |
| `auth abilities <NODE_ID>` | SHIPPED | — | |
| `auth exec <NODE_ID> -- <CMD>` | SHIPPED | — | |
| `auth agents` | SHIPPED | — | |
| `auth events [--node]` | SHIPPED | — | Raw SSE tail. See §8 — promote to first-class `events`. |
| `auth device-remove <NODE_ID> [-y]` | SHIPPED | — | Duplicates `device remove`; consider consolidating. |

**Review note:** solid. One papercut — `auth` mixes *identity* (login/whoami/pair) with *fleet reads*
(devices/abilities/exec/agents) that duplicate `device`/`agent`/`ability`. Candidate for a cleanup
pass so `auth` = identity only.

---

## SECTION 2 — Agent (network actors)

### Shipped (local / daemon-owned)
| Command | Status | Pri | Notes |
|---|---|---|---|
| `agent add <NAME> [--type] [--model] [--timeout]` | SHIPPED | — | |
| `agent list` | SHIPPED | — | |
| `agent remove <NAME>` | SHIPPED | — | |
| `agent prune` | SHIPPED | — | |
| `agent doctor <NAME>` | SHIPPED | — | |
| `agent send <NAME> <PROMPT> [--follow] [--resume] [--session-id]` | SHIPPED | — | |
| `agent session new/list/show/append/end` | SHIPPED | — | Memory dimension. Good. |
| `agent abilities [--agent]` | SHIPPED | — | |
| `agent mcp` | SHIPPED | — | Binds upstream MCP tools as abilities. |
| `agent set <NAME> [--model]` | SHIPPED | — | |
| `agent publish <NAME> [--json]` | SHIPPED | — | Dry-run preview. |
| `agent refresh [--agent]` | SHIPPED | — | |
| `agent chat-history <NAME>` | SHIPPED | — | |
| `agent discuss` | SHIPPED (DEPRECATED) | P2 | Migrating to `mission discuss`. Finish the migration, then drop. |

### Missing (agent as a *network citizen* — the spec's "first-class" claim)
| Command | Status | Pri | Backing | What it's for |
|---|---|---|---|---|
| `agent resolve <ura>` | **MISSING** | P1 | roadmap (`ResolveAgent`) | URA → agent descriptor. Today only nodes resolve, not agents. |
| `agent register <ura> ...` | **MISSING** | P1 | roadmap (`RegisterAgent`) | Give an agent a network identity, not just a local wrapper. |
| `agent migrate <name> --to <node>` | **MISSING** | P2 | roadmap (`MigrateAgent`) | Move an agent (with memory) across owned nodes. |
| `agent trust <node\|agent>` | **MISSING** | P0 | message exists; callable ability unverified (`identity.get_trust`) | See §4. |

**Review note:** today "agent" means "a local wrapper around Claude/Codex." The product spec says an
agent is "a first-class citizen with identity, memory, relationships, reputation, economic capability."
The `register/resolve/migrate` trio is what closes that gap. **Discussion needed: is agent-identity a
v1.1 commitment or later?** That decides P1 vs P2.

---

## SECTION 2.5 — Ability ownership & the two GET routes (foundational — read before §3)

> Triggered by the question *"who does an ability belong to — agent? device? user?"* The answer
> reshapes the GET axis and underpins §3.6 (family) and §4.5 (authorization). Verified against
> `EasyNet-Axon/document/concepts/ONTOLOGY_AGENT_ABILITY.md`.

### Ownership widened in URA v4.1.5 — it is no longer "an ability belongs to an agent"

- **Old (v4.1.4):** *"Ability always belongs to exactly one Agent."*
- **Current (v4.1.5, ontology:148):** **"Ability belongs to exactly one of `{user, agent, hub, resource}`."**
  Cardinality (exactly one owner) is non-negotiable; the owner *kind* widened. URA grammar is uniform:
  `ability/<owner-id>.<name>` — only the static type of `<owner-id>` changes.

| Owner kind | When | Example URA |
|---|---|---|
| **user** | lifecycle abilities anchored to a human; outlive whichever agent the user runs | `ability/alice.pages.publish` |
| **agent** | skills inseparable from a specific agent process | `ability/alice.claude.skill.summarize` |
| **hub** | cross-network coordination / transport adapters owned by the singleton Hub | `ability/01HUB.federation.discover` |
| **resource** | scoped to a resource instance; registered at create, unregistered at delete | `ability/alice.papers.page.fetch` (owner-id is a *resource* id, not an agent) |

- **`device` is NOT an owner.** The six URA roles are `{user, device, agent, ability, hub, resource}`,
  but `subject ∈ {user, device, resource}` is a hard admission rule — **device is the *host/subject*
  (where capability runs / what is acted upon), never the *owner* (who answers for receipts,
  visibility, authorization).** Owner and host are orthogonal: `ability/alice.pages.publish` is owned
  by user `alice` but may host on any of her devices.

### Consequence: the two GET routes (and why it's `teach`/`learn`, not `pull`)

Because the capability owner is a first-class identity that answers for the ability, GET has **two
owner-driven routes** — not a file download:

| Route | Verbs | Semantics | Capability leaves owner? | Owner consent |
|---|---|---|---|---|
| **A — default: remote invoke** | `invoke` | use it where it lives; get result + receipt | ❌ never | implicit (it's public/scoped) |
| **B — optional: teach/learn** | owner `teach` → learner `learn`/`study` | learner *acquires* the ability and becomes its **new owner** (`ability/<learner>.<name>`) | ✅ a copy is conferred | **explicit & active** |

- **Why `teach`, not `pull`:** `pull` is the consumer *taking*; it implies the capability is a file
  and contradicts "capability never leaves its owner." `teach` is the owner *actively conferring* — the
  initiative stays with the owner. Default `InstallPolicy.allow_transferred_code = false`
  (capability.proto:237): an ability is **not learnable unless the owner teaches it.**
- **No double-owner problem:** after `learn`, the learner owns *their copy* under their own URA. The
  original keeps its owner. "Exactly one owner" holds for each.
- **`learn` is already in the ontology:** `acquire(ability) ≡ invoke(self, self, meta.acquire, {ability})`
  (ontology:220). `forget` ≡ `meta.forget`. CLI just needs to surface them — and add the cross-owner
  `teach` half.
- **`study` = read-only learn:** acquire the ability's contract/schema/behaviour to *understand* it,
  without a runnable copy. For "I want to see how this works" without installing.
- **Safety gate exists:** transferred code runs under `InstallPolicy.execution_mode ∈
  {sandbox_first, host, docker_only}` + `require_consent` (capability.proto:236) — ties Route B
  directly to the PROTECT axis (§4.5).

**One-liner:** *the capability owner is `{user|agent|hub|resource}`; the device is only the host.
GET route A (invoke) keeps the capability home; GET route B (teach→learn) lets an owner deliberately
confer it, after which the learner is its new owner — never a silent `pull`.*

---

## SECTION 3 — Ability lifecycle & discovery

### Shipped
| Command | Status | Pri | Notes |
|---|---|---|---|
| `ability new <NAME> [--lang]` | SHIPPED | — | |
| `ability validate <PATH>` | SHIPPED | — | |
| `ability list [--format]` | SHIPPED | — | Lists *your federation's* abilities (not cross-owner search). |
| `ability show <NAME> [--node] [--format]` | SHIPPED | — | |
| `ability deploy <PATH> --node [--version]` | SHIPPED | — | |
| `ability uninstall <NAME> [--node] [--install-id] [-y]` | SHIPPED | — | |
| `ability invoke <NAME> [--node] [--json] [--input]` | SHIPPED | — | The core verb. Auto-routes. Good. |
| `ability exec <NODE_ID> -- <CMD>` | SHIPPED | — | Ad-hoc ephemeral ability. |

### Missing — DISCOVERY (P0)
| Command | Status | Pri | Backing | What it's for |
|---|---|---|---|---|
| `discover "<intent>"` / `ability search <query>` | **MISSING** | **P0** | proto (`ListFederatedNodes` w/ `ability_filter`/`tag_filter` federation.proto:292–305) + roadmap (semantic/pgvector ranking) | Ranked callable abilities **across owners**, by trust/price/latency. **The single highest-leverage command in this doc.** |
| `ability resolve <name>` | **MISSING** | P1 | proto (`ResolveCapability` capability.proto:374) | Resolve an ability name → best concrete endpoint. |

### Missing — GET ROUTE B: teach / learn / study (P1 — **ontology-only**: no handler AND no CLI)
> See §2.5. The "owner deliberately confers a capability" route — NOT `pull`. **Reality check from
> audit:** `meta.acquire`/`meta.forget` are defined in the ontology but have **no runtime handler** —
> only `meta.describe` + `meta.list_abilities` are registered today. So this is a two-layer gap
> (handler + CLI), not just a missing verb. `InstallCapability` is itself restated as `meta.acquire`
> (RFC-001 mapping §, AAL §3.4) — i.e. teach/learn and capability-install are the *same* primitive.

| Command | Status | Pri | Backing | What it's for |
|---|---|---|---|---|
| `ability teach <ability> --to <learner> [--with-assets]` | **MISSING** | P1 | proto-ability (`meta.acquire` ← `InstallCapability`) + gate `InstallPolicy` capability.proto:235–239 | Owner actively confers a learnable copy. Default-off (`allow_transferred_code=false`, :237); explicit opt-in. |
| `ability learn <ability> --from <owner>` | **MISSING** | P1 | ontology-only (`meta.acquire` ontology:220 — no handler yet) | Learner acquires a taught ability; becomes its new owner under its own URA. |
| `ability study <ability>` | **MISSING** | P2 | ontology-only (read-only `meta.acquire`) | Acquire the *contract/behaviour* to understand it — no runnable copy. |
| `ability forget <ability>` | **MISSING** | P2 | ontology-only (`meta.forget` ontology:221 — no handler yet) | Drop a learned ability (unlearn). |

### Missing — RELEASE PIPELINE (P1, message-backed; callable abilities require confirmation)
| Command | Status | Pri | Backing | What it's for |
|---|---|---|---|---|
| `ability sign <pkg>` | **MISSING** | P1 | proto (`SignPackage` capability.proto:121) | Trust-root counter-signature for distribution. |
| `ability promote <name> --channel stable` | **MISSING** | P1 | proto (`PromotePackage` capability.proto:137) | Canary → stable channel advance. |
| `ability rollout <name> --stages ...` | **MISSING** | P2 | proto (`CreateRollout` capability.proto:165) | Staged rollout with SLO gates. |
| `ability rollback <name>` | **MISSING** | P1 | proto (`RollbackCapability` capability.proto) | Revert a bad version. |
| `ability consent grant\|revoke <ability>` | **MISSING** | P1 | message exists; callable `capability.*`/`consent.*` ability unverified | User approval workflow for sensitive abilities. |

**Review note:** the CLI flattens a real release pipeline (`publish→sign→promote→rollout→install→
activate→rollback/revoke + consent`) down to `deploy`/`uninstall`. Fine for a solo owner; insufficient
for *publishing an ability strangers depend on*. **Discussion: do we expose the full pipeline, or a
curated subset (`sign`, `promote`, `rollback`)?** I lean curated subset for v1.1.

---

## SECTION 3.5 — Ability Adapters: wrapping the existing world (SHIPPED — under-marketed)

> **Verified strength I under-sold in the first pass.** `AbilityExec` (`src/core/ability_spec.rs:424`)
> is a four-variant adapter enum. EasyNet can already turn three classes of *existing* software into a
> network-addressable ability with **zero glue code**, plus compose abilities into new abilities. This
> is a major adoption lever — you don't rewrite your tools, you wrap them.

| "Turn X into an ability" | Status | Backing | How |
|---|---|---|---|
| **CLI / shell command → ability** | **PARTIAL** | `Shell(ShellExec)` (ability_spec.rs:424) | The executor exists, but **there is no CLI verb that scaffolds a shell ability** — `ability new` does *not* emit an `[exec]` block, so a user must hand-edit the manifest to add `kind=shell argv=[…]`. Code-complete, not user-facing. Argv form bypasses `sh -c` (anti-injection). |
| **HTTP API → ability** | SHIPPED | `Http(HttpExec)` | `agent new-ability api add <url>` — args templated into URL/headers/body, values **auto URL-encoded**, no subprocess, no argv-injection surface. |
| **OpenAPI operation → ability** | SHIPPED | `Http(HttpExec)` | `agent new-ability from-openapi <spec>` (subcommand `name = "from-openapi"`, agent_new_ability.rs:84) — generate an ability straight from an OpenAPI spec. |
| **MCP tool → ability** | SHIPPED | `Mcp(McpExec)` + `agent mcp` | Bind an upstream MCP server's tool catalogue onto an agent as *deterministic* abilities; preserves MCP `tools/call` response shape, no shell/chat translation. |
| **Ability composed from abilities → ability** | SHIPPED | `Eal(EalExec)` | A published ability whose body is a small EAL program orchestrating existing abilities — **reuses the same EAL the operator already runs, no second orchestration surface.** This is the canonical answer to "sub-abilities" (see §3.6). |

**Gaps (ergonomics, not capability):** the import verbs are buried under `agent new-ability {http,
openapi,mcp}`. Discoverability would jump with first-class spellings: `ability from-cli`,
`ability from-openapi <spec>`, `ability from-mcp <server>`. The capability is done; the *front door*
is hidden. **P2 — naming/UX, not protocol.**

---

## SECTION 3.6 — Ability structure: group → family-tree → projection (the right answer)

> This section records a design debate, because the conclusion is non-obvious and the two tempting
> extremes are both wrong.

**The reasoning chain:**

1. **First proposal — a flat `Group` object (too weak).** A label-bag of abilities you can deploy/
   grant/discover as a unit. Rejected: it's just a tag with extra steps; it doesn't capture that
   abilities have real *structure*.
2. **Counter-proposal — a `family tree` where calling follows the tree, not a flat list (too strong).**
   The intuition: "a software CLI is born a tree (`git remote add`), so abilities should be a family,
   and invoking shouldn't be flattened." This is half-right and half a trap (below).
3. **Resolution — structure is a *projection / scope*, never an *address* or *call semantics*.**

**Why "tree as call/address" is a trap (the PM rebuttal):**

- **It conflates the display tree with the call graph.** `git remote add`'s tree is *navigation for
  humans* — `add` isn't an independently-callable thing, it's a branch of `remote`. But an EasyNet
  ability is a network first-class citizen a *stranger* can call directly. Making `aris.review` a
  "child" of `aris.research` forces the unanswerable question: can a stranger bypass the parent and
  call the child? If yes → the tree was only cosmetic (≡ flat namespace). If no → the child isn't an
  ability, it's the parent's internal implementation, and that's exactly what `Eal` composition is for.
- **It welds org-structure into the address.** URA is an address. Put `research/review/verify` into it
  and *reorganizing the family = changing addresses = breaking every external caller.* `agent_id.rs`
  deliberately caps names at two segments (`tenant/name`), reserves multi-level `a/b/c` as
  `ReservedV2`, and forbids `.` inside a name (reserves it for `agent.verb`) — the codebase is already
  protecting "addresses stay flat and stable." A family tree on the URA fights that on purpose.
- **Flattening the call surface is EAL's whole thesis.** The platform keeps the *invocation* primitive
  singular and pushes structure into EAL *programs*. The tree lives in the **program** (dynamic,
  versionable, replaceable), not in the **address** (must stay flat and stable).

**Why the intuition is nonetheless right (the half to keep):** a flat `ability list` *does* throw away
real structure. Abilities cluster into families, and an owner *should* be able to discover a family and
govern a family as a unit. The fix is to put the tree where it belongs.

**The layering — each row maps to an existing primitive:**

| Dimension | Tree? | Lives in | Status |
|---|---|---|---|
| **Addressing** (URA) | ❌ flat (`tenant/name`) | protocol | locked, correct (agent_id.rs:110) |
| **Invocation** (`invoke`) | ❌ flat (one primitive) | protocol | shipped, correct |
| **Composition** (ability built from abilities) | ✅ tree (dynamic) | **inside `Eal` impl** | ✅ shipped |
| **Discovery / display** ("this family of abilities") | ✅ tree (for humans) | **CLI / `discover` projection** | ❌ **MISSING — the real gap** |
| **Authorization** ("grant a whole family at once") | ✅ tree (policy scope) | **`policy` condition** | ❌ MISSING |

**Proposal (replaces the weak flat-group idea):**

- **No new first-class `Group` object. No change to URA.**
- Add an *optional* projection-only `family` (a.k.a. `parent`) metadata field on the ability manifest —
  it participates in **rendering and policy scope only**, never in addressing or routing.
- Let `discover` / `ability list` render it as a tree:
  ```
  easynet discover "research" --tree
  aris/                      (family — display only, not an address)
  ├─ aris.research   [public]
  ├─ aris.review     [scoped: reviewer-agents]
  └─ aris.verify     [public]
  ```
- Let `policy` conditions scope by family prefix — *grant a whole family in one rule* (ties to §4.5):
  `allow action=invoke where ability.family == "aris" && trust_level >= STANDARD`
- Real sub-ability composition stays in **`Eal`** (already shipped); it is **not** exposed as separate
  network nodes.

| Command | Status | Pri | Backing | What it's for |
|---|---|---|---|---|
| `discover --tree` / `ability list --tree` | **MISSING** | P1 | cli-only (projection over `family` metadata) | Render abilities as families for humans — without making families addressable. |
| `policy` condition `ability.family == "..."` | **MISSING** | P1 | proto (`PolicyRule.condition` policy.proto:102) | Authorize/deny a whole family in one rule. |

**One-liner:** *the tree is a tree of discovery and authorization, not of calling and addressing.* Keep
the call surface flat; let projection (display) and scope (policy) supply the family feel — so you get
readability and governance without welding team structure into the address.

---

## SECTION 3.7 — URA namespace segment: a designed-but-unimplemented family dimension (P1, evidence-grade finding)

> **Decision (locked):** the `namespace` segment of an ability URA will be **formally implemented as
> the Ability Family dimension** — not deleted. The structure the team needs is already in the address;
> it was simply never parsed, stored, indexed, or exposed. This is *completing a dangling protocol
> dimension*, not inventing a new concept.

### The evidence (authoritative source: `EasyNet-Axon/sdk/rust/src/ura.rs`)

The documented ability URA shape is **three dot-separated segments**:

```
ability   easynet:///r/<realm>/ability/<owner>.<namespace>.<ability-id>
e.g.      easynet:///r/localhost/ability/u-9f4.frontend-engineer.chat
          easynet:///r/localhost/ability/hub.federation.resolve   ← namespace = "federation"
```

But the parser **validates the three segments exist and then discards the middle one**:

- `ParsedURA` (sdk/rust/src/ura.rs:117–127) has fields `user_id`, `device_id`, `agent_id`,
  `ability_id`, `path`, `raw` — and **no `ability_namespace` field**. The whole ability tail collapses
  into `ability_id: String`.
- The only field literally named `namespace` is `Option<ResourceNamespace>` — an enum of
  `fs|process|pty|shell|http` (ura.rs:90–96) that belongs to **resource** URAs, not abilities. The word
  "namespace" is already taken by an unrelated concept.
- Parse enforces shape only: `AbilityBadShape → "ability tail must have three non-empty dot-separated
  parts"` (ura.rs:165). After the check, the segment is gone.
- The builder confirms it: `Ura::ability(realm, user_id, agent_id, ability_id)` (ura.rs:212) just
  string-concats `{user_id}.{agent_id}.{ability_id}`; the hub-owned path (ura.rs:219) does
  `split_once('.')` to peel a namespace and **immediately re-concatenates it** — never retained as a
  queryable dimension.

**Conclusion: the namespace segment is a ghost dimension** — mandated by the format, dropped by the
implementation. The user's observation ("I don't see namespace used anywhere in Axon or the CLI") is
*correct at the code level*: it is never structurally parsed, has no field, no index, no query entry.

### Why "implement as family" (not delete) is the right call

`hub.federation.resolve`, `hub.federation.join`, `federation.watch_membership` — the hub-owned abilities
**already share `federation` as a de-facto family namespace**. The dimension isn't speculative; it's in
active use on the hub side, just unstructured and unavailable to user-owned abilities. The address is
already a tree (`realm / user / agent / ability`); we are exposing the tree that exists, not building one.

### What "implement" concretely means

| Work item | Status | Pri | Where | What it does |
|---|---|---|---|---|
| Add `family` (namespace) to `ParsedURA` | **MISSING** | P1 | `sdk/rust/src/ura.rs:117` (+ go/python/swift/node SDK parity) | Structurally parse & retain the middle segment instead of folding it into `ability_id`. |
| Structured `owner / agent / verb` split | **MISSING** | P1 | same | Stop slicing strings ad-hoc; expose the levels the URA already encodes. |
| `discover` / `ability list` fold by family | **MISSING** | P1 | CLI | Default to a `family → verb` tree (extends §3.6's agent-fold with the real middle level). |
| `policy` scope by `ability.family == "federation"` | **MISSING** | P1 | proto `PolicyRule.condition` (policy.proto:102) | Authorize/deny a whole family in one rule — ties to §4.5. |
| Migrate user-owned abilities to declare a family | **MISSING** | P2 | ability.toml | Today user abilities are effectively `<user>.<agent>.<verb>`; give authors a real family slot like hub already has. |

### Review note (this is a root-cause finding, not a feature request)

Three rounds of this review converged here. The "should we add a `group` object?" question (too weak)
and the "is the dot a family tree?" question (right instinct, wrong axis) both resolve to the same fact:
**the family dimension was designed into the URA and left unimplemented.** Don't add a parallel grouping
concept — finish this one. Implementing it makes §3.6's discovery-fold and §4.5's policy-scope *fall out
for free*, because both can key off a field that finally exists.

**Discussion needed:**
- Naming: keep the URL segment word "namespace", or rename the *concept* to "family" while leaving the
  wire segment position unchanged? (I lean: wire stays `<owner>.<namespace>.<ability-id>`; we *call* it
  family in UX and docs to avoid colliding with `ResourceNamespace`.)
- Migration: do user-owned abilities get an explicit family slot now (breaking the implicit
  `<user>.<agent>.<verb>` reading), or do we treat `agent` as the family for user-owned and reserve the
  explicit slot for hub-owned? This is the one real design fork.

---

## SECTION 3.8 — Audit: namespace hierarchy, recursion, permission binding, and the OOP graft (evidence-grade)

> **The question asked:** "Is the layering + recursion of namespace, its binding to the permission
> system, and how OOP structure plugs in — actually *designed*?"
> **The honest answer, from the code:** **No. All three are dangling.** The dots in the URA *look* like
> an object graph, but nothing behind them is structured, recursive, or enforced. Below is exactly how
> far each got, with file:line, split into `[IMPLEMENTED]` / `[DOC-ONLY]` / `[DEFERRED-REJECTED]` /
> `[ABSENT]`. This is the deepest finding in the review — it's not a missing command, it's a missing
> *model*.

### A. Namespace layering & recursion — `[DEFERRED-REJECTED]`, flat by design

| Claim | Status | Evidence |
|---|---|---|
| Ability namespace is a single flat segment | `[IMPLEMENTED]` | `splitn(3, '.')` + reject `agent_id.contains('.')` → `AbilityBadShape` (Axon `sdk/rust/src/ura.rs:451,460`). Exactly 3 parts; the middle cannot nest. |
| Multi-level namespace `a/b/c` is rejected | `[DEFERRED-REJECTED]` | `agent_id.rs` → `ReservedV2 { feature: "multi-level namespace (\`a/b/c\`)" }`. Doc `AGENT_IDENTITY.md:290`: *"not currently planned; reserved to avoid binding."* |
| Recursive / nested namespace semantics (sub-namespace under namespace) | `[ABSENT]` | No `hierarchy`/`nested`/`recursive` design for namespace anywhere. The only "recursion" in the docs is the **open question** of *recursive Agent* (`ONTOLOGY_AGENT_ABILITY.md:474–494`) — Agent-composed-of-Agents — which is explicitly *unresolved* and is about Agent structure, not namespace. |
| Any place that *does* support arbitrary depth | `[IMPLEMENTED]` (but elsewhere) | **Only `resource` URA path**: `resource/<owner>/<a/b/c/...>` — parser does `split_once('/')` then keeps the path verbatim (`ura.rs:475–489`). This is the system's one true recursive segment, and it's a data path, not a capability tree. |

**Verdict:** namespace hierarchy/recursion is **not designed** — it's a *deliberately flat 3-tuple* with multi-level explicitly reserved-and-rejected. The recursion you'd expect for an object graph lives only in `resource` paths (for files/objects), and the recursion the ontology *gestures at* (recursive Agent) is an open question with no implementation.

### B. Namespace ↔ permission binding — `[ABSENT]`, fully decoupled

This is the sharper finding. The authorization system **cannot reference the family dimension at all**:

| Mechanism | Status | Evidence |
|---|---|---|
| `PolicyRule.condition` expression (`"trust_level >= STANDARD && ..."`) | `[DOC-ONLY]` | The `condition` string field exists (`policy.proto:102`) but **is never read**. Real admission is hardcoded to two checks: `action.contains("install") → need Privileged`, `action.contains("admin") → need Elevated` (`runtime-rs/src/runtime/resilience.rs:711,715`). No CEL, no expr engine, no variable binding. |
| Referencing `ability.namespace` / `ability.family` in a rule | `[ABSENT]` | Even if `condition` were evaluated, `ability_name` is an **opaque string** in the admission path; it is never parsed into `<owner>.<namespace>.<id>`. There is no `family` variable to bind. |
| Prefix / wildcard authorization (`federation.*`) | `[ABSENT]` (capability exists, unused) | A `wildcard_match()` exists (`runtime-rs/src/runtime/mod.rs:589`) and *could* match `federation*` → `federation.join`, but **it is never wired into policy gating**. Dead capability. |
| Consent `scope` ("execute"/"install"/"access_camera") bound to family | `[ABSENT]` | `scope` is a **free-form, unvalidated string** stored in `ConsentView` (`services/capability/rpc_consent.rs:85`); never checked against an invocation or an ability name. Informational, not a boundary. |
| Ability `visibility` PUBLIC/PRIVATE/SCOPED + `authorized_agents` | `[DOC-ONLY]` | Defined in ontology (`ONTOLOGY_AGENT_ABILITY.md:107–122`) but **absent from proto** — `CapabilityDescriptor` (`types.proto:746`) has no `visibility`/`authorized` fields, and runtime `CapabilityInstallRecord` (`state/capability.rs:30`) doesn't track them. SCOPED is 0% implemented. |
| Hierarchical / cascading authorization (deny `alice.vision.*` ⇒ deny children) | `[ABSENT]` | Policies are a flat `(tenant, action, node, ability_name)` key. No parent-scope lookup, no scope-tree walk, no family-wide grant. Each ability is authorized as a singleton, by exact string. |

**Verdict:** the permission system and the URA family dimension are **completely decoupled**. The dots are *structural in the address only*; authorization treats `ability_name` as a flat opaque string. You **cannot** today say "grant agent B all of `aris.*`" — there is no field to match, no evaluator to match it, and no inheritance to cascade it. This directly undercuts §3.7's "policy-scope falls out for free": it only falls out *if* both the family parse (§3.7-A) **and** a condition evaluator (§3.8-B) get built. They are prerequisites, not freebies.

### C. The OOP graft — `[DOC-ONLY]`, one analogy, no model

The intuition "this should be an object graph" is sound and *partially present*, but never formalized:

| OOP notion | Maps to (intended) | Status | Evidence |
|---|---|---|---|
| **Method** | Ability (a verb on an Agent) | `[IMPLEMENTED]` | `invoke(caller, callee, ability, args)` is literally method dispatch; `<agent>.<verb>` is `receiver.method`. |
| **`toString()` / base method** | A default callable every object has | `[DOC-ONLY]` | The `chat` ability is described as *"Analogous to `Object.toString()` in Java — every agent exposes this"* (`gallery/case01-aris/abilities/chat/ability.json:5`). But there is **no base-Agent type** that *guarantees* `chat`; it's a convention, not an inherited member. |
| **Encapsulation** (private members) | Skill = PRIVATE ability | `[DOC-ONLY]` | Ontology says Skill = `visibility=PRIVATE`. But visibility isn't in proto/runtime (§3.8-B), so the private/public boundary is **not structurally enforced** — it's an honor system. |
| **Composition** (object made of objects) | Recursive Agent; `Eal` ability composing sub-invocations | `[IMPLEMENTED]` (composition) / `[DOC-ONLY]` (recursive Agent) | `Eal(EalExec)` lets an ability be implemented by composing others — real composition. But "Agent composed of Agents" is the *open question* (`ONTOLOGY:474`), undecided. |
| **Inheritance** (family shares members) | Family/namespace shares abilities or policy | `[ABSENT]` | No type hierarchy, no member inheritance, no "all `aris.*` inherit X." The family segment doesn't even parse (§3.7). |
| **Polymorphism** (same call, different impl) | Same ability name across agents (`reviewer-gpt.review` vs `aris.review`) | `[IMPLEMENTED]` (de facto) | Two agents can both expose `review`; caller picks the receiver. This is duck-typing by ability name — works, but unmodeled (no interface/contract type asserting they're substitutable). |
| **`this` / `self`** | The 7-tuple's `subject` ≠ `callee` | `[IMPLEMENTED]` (partial) | `subject` vs `callee` distinction (`types.proto:331`) is exactly "the object being acted on may differ from the receiver" — a real `this`/target split. Underused at CLI. |

**Verdict:** Axon has the *bones* of an object model — method dispatch, a `subject`/`callee` (`this`) split, real composition via `Eal`, de-facto polymorphism by name — but **no declared type system**: no base-Agent, no interface/contract, no inheritance, and encapsulation is unenforced because `visibility` isn't implemented. The `Object.toString()` line is the only explicit OOP statement in the whole codebase, and it's a comment in a demo manifest.

### What this means (and what to write down as the decision)

The three questions are really **one** question: *does EasyNet have an object model, or just an object-shaped address?* Today it's the latter. To make the family dimension (§3.7) actually pay off, three things must land **in order**, because each depends on the last:

| Step | Builds | Without it… |
|---|---|---|
| 1. **Parse the family** (§3.7-A) | `family` field in `ParsedURA` (+ SDK parity) | nothing can reference a family |
| 2. **A condition evaluator** that can read `ability.family`, with prefix/wildcard (`aris.*`) | a real `PolicyRule.condition` engine (replace the 2 hardcoded checks in `resilience.rs`) | family-scoped authorization is impossible; §3.7's "free policy scope" is fiction |
| 3. **Implement `visibility` + `authorized_agents` in proto/runtime** | enforced encapsulation (the OOP private/public boundary) | Skill-vs-public is an honor system; SCOPED is vapor |

**Decision to record:** treat family + permission + OOP as a *single coherent model project*, not three commands. Recommended scope ladder:
- **v1 (enforce what's claimed):** implement `visibility`/`authorized_agents` (step 3) so encapsulation is real; keep family flat (one level).
- **v1.x (make family governable):** steps 1–2 — parse family, build a minimal condition evaluator supporting `trust_level`, `ability.family`, and prefix match. This unlocks "grant all of `aris.*`."
- **v2 (only if a real use case forces it):** revisit recursive Agent (`ONTOLOGY:474`) and multi-level namespace (`ReservedV2`). Do **not** build these speculatively — the `resource` path already absorbs arbitrary-depth data needs.

**Hard rule to keep:** the object model is for **encapsulation, discovery, and authorization** — never for **addressing or routing**. `subject`/`callee` stays the `this` split; `invoke` stays a flat single dispatch. An object graph in the address is the file-system-deep-path mistake all over again (§3.6).

**Open forks for the team:**
1. Is there a **base-Agent contract** (every Agent guarantees `chat`/`describe`), or stays convention? (decides whether `toString()` analogy becomes real)
2. Does **polymorphism** get a contract type (an `interface`/`ability-spec` that several agents declare they implement, so `discover` can rank substitutable providers), or stays duck-typed-by-name?
3. Is the **condition evaluator** a full expression language (CEL-like) or a deliberately tiny matcher (trust + family-prefix + node-label)? I lean tiny-first — the 2 hardcoded checks today prove the appetite is small.

---

## SECTION 4 — Trust (P0, the missing spine)

> **Corrected after audit.** `TrustLevel` is *not* only in federation TLS plumbing — it is a field on
> `NodeDescriptor` (types.proto:671) and is **actively used in the authorization path**
> (`resilience.rs:714` hardcodes "install requires PRIVILEGED, admin requires ELEVATED"). What's missing
> is the **user-facing surface**: no `trust show`/`trust set` verb, and the get/set are now intended
> abilities (`identity.get_trust`/`identity.set_trust`, RFC-001) rather than RPCs. The data messages
> exist, but callable ability publication must be confirmed in Axon before this can be treated as
> cheap CLI exposure.
>
> (original note) `trust` today also appears in federation TLS cert plumbing. There is no user-facing
> trust surface, despite Axon modeling it fully.

| Command | Status | Pri | Backing | What it's for |
|---|---|---|---|---|
| `trust show <agent\|node>` | **MISSING** | **P0** | message exists; callable ability unverified (`identity.get_trust`) | Trust level (UNTRUSTED→PRIVILEGED), reputation, history. Answers *"should I call this stranger?"* |
| `trust set <node> --level elevated` | **MISSING** | **P0** | message exists; callable ability unverified (`identity.set_trust`) | Operator raises/lowers trust on a peer. |
| `trust list` | **MISSING** | P1 | message exists; callable ability unverified | Who do I trust, and at what level? |

**Review note:** you cannot run a marketplace where strangers transact without a trust display. Axon
already has the trust ontology and messages, but the callable surface is the load-bearing question:
until `identity.get_trust` / `identity.set_trust` are confirmed as published abilities, this is a
protocol/ability-publication dependency, not pure CLI exposure.

---

## SECTION 4.5 — Permission & Authorization (P0, the OTHER missing spine)

> **Verified gap.** Axon ships a *complete* authorization stack — a policy engine, consent grants,
> temporary overrides, and ability visibility — and the CLI exposes **none of it** as a management
> surface. There is no `policy`, `permission`, `consent`, or `grant` command group. This is the
> control plane that protects **service, data, and resources**: who is allowed to call what, see what,
> and run what. Without it, "the capability never leaves your machine" is a slogan, not an enforced
> guarantee the owner can configure.

### The model Axon already has (messages exist; callable abilities still need confirmation)

- **Policy engine** — `PolicyRule { effect: allow|deny, action, condition, priority }` where `condition`
  is an expression like `"trust_level >= STANDARD && node.labels.region == 'cn-east'"`. Grouped into a
  `PolicySet`. (policy.proto:100–117). This is the *rules* layer: declarative allow/deny.
- **Consent** — `GrantConsent/RevokeConsent/ListConsents` with `scope: "execute" | "install" |
  "access_camera" | ...` (capability.proto:391–441). This is the *resource* layer: per-scope
  permission grants — exactly "an ability may access the camera," "an agent may install on this node."
- **Temporary override** — `CreateOverride/RevokeOverride`, use case literally *"temporarily allow
  ability X on node Y for debugging"* (policy.proto:166–191). The break-glass / time-boxed grant.
- **Ability visibility** — PUBLIC / PRIVATE(=Skill) / SCOPED(authorized_agents) from the ontology. The
  *who-can-even-see-this* layer.
- **Install-time guards** — `require_consent`, `allow_transferred_code` (capability.proto:239–240).

### Missing commands

| Command | Status | Pri | Backing | What it's for |
|---|---|---|---|---|
| `policy list` | **MISSING** | **P0** | message exists; callable `policy.*` ability unverified | What rules govern my node/abilities right now? |
| `policy show <id>` | **MISSING** | P1 | message exists; callable `policy.*` ability unverified | Inspect one policy set's rules. |
| `policy create --rule "deny action=exec unless trust>=STANDARD"` | **MISSING** | **P0** | message exists; callable `policy.*` ability unverified | Author an allow/deny rule. The core authz verb. |
| `policy update <id> ...` | **MISSING** | P1 | message exists; callable `policy.*` ability unverified | Edit rules in place. |
| `policy simulate <invocation>` | **MISSING** | P1 | message exists; callable `policy.*` ability unverified | Dry-run: *would this call be allowed, and which rule fires?* Huge for debugging. |
| `policy why <invocation-id>` | **MISSING** | P1 | message exists; callable `policy.*` ability unverified | Explain a past allow/deny decision (the `deny_reasons` codes). |
| `policy override create --ability X --node Y --ttl 1h --effect allow` | **MISSING** | P1 | message exists; callable `policy.*` ability unverified | Break-glass / time-boxed grant for debugging. |
| `policy override revoke <id>` | **MISSING** | P1 | message exists; callable `policy.*` ability unverified | Close the break-glass window. |
| `permission grant <agent> --scope execute --ability X` | **MISSING** | **P0** | message exists; callable `capability.*` ability unverified | Let a *specific caller* use a scoped ability/resource. |
| `permission revoke <consent-id>` | **MISSING** | **P0** | message exists; callable `capability.*` ability unverified | Withdraw access. |
| `permission list [--agent] [--ability]` | **MISSING** | P1 | message exists; callable `capability.*` ability unverified | Who can touch what, at what scope? The audit view. |
| `ability set-visibility <name> --public\|--private\|--scoped <agents>` | **MISSING** | P1 | message exists; callable ability unverified | Control who can even *discover/see* an ability. |

**Review note (this is the important one):** trust (§4) answers *"how much do I trust this stranger?"*
Permission (§4.5) answers *"what is this caller actually allowed to do to my service, data, and
resources?"* — and it must work at **three granularities the model already supports**:
1. **rule-level** (`policy` — declarative allow/deny by condition),
2. **resource/scope-level** (`permission`/consent — "may access camera", "may execute ability X"),
3. **visibility-level** (`ability set-visibility` — who can see it at all).

A marketplace where I can't say *"agent B may call ability X but only read, not write, and only if
trust ≥ STANDARD"* is not safe to open to strangers. **This is co-equal P0 with Trust — they are the
two halves of the same control plane.** The messages exist, but CLI work must be sequenced behind
confirmation that Axon publishes the callable `policy.*` / `capability.*` abilities; only after that
does this become mostly CLI exposure plus `--rule "..."` ergonomics design.

**Discussion needed:**
- Do we ship `policy` and `permission` as **two command groups** (rules vs scoped grants), or unify
  under one `permission`/`access` noun? I lean two groups — they map to two distinct Axon services.
- Condition-authoring UX: raw expression string (`--rule "trust_level >= STANDARD"`) vs guided flags
  (`--min-trust standard --action exec`). Raw is powerful; guided is safe. Probably both.
- Is `policy simulate` / `policy why` a launch feature? It's the difference between "permission denied"
  being debuggable vs a black box. Strong argument for P0/P1.

---

## SECTION 5 — Economics (P0 by pitch, roadmap by proto)

> Verified: **no** `Agreement`/`Wallet`/`Payment`/`Escrow` message exists in proto today. This whole
> section is **roadmap-backed** — it needs Axon protocol work first (EasyNet-Ledger, Agreement primitive).

| Command | Status | Pri | Backing | What it's for |
|---|---|---|---|---|
| `wallet balance` | **MISSING** | P0 | roadmap (EasyNet-Ledger) | "What's my credit balance?" |
| `wallet history` | **MISSING** | P0 | roadmap | Per-invocation cost ledger (pairs with `invocation list`). |
| `agreement propose <agent> --sla --price` | **MISSING** | P1 | roadmap (Agreement: `Propose`) | Sign an SLA/pricing contract with a provider. |
| `agreement accept\|reject\|counter <id>` | **MISSING** | P1 | roadmap (`Accept`/`Reject`/`Counter`) | Negotiation lifecycle. |
| `agreement list \| show <id>` | **MISSING** | P1 | roadmap (`Query`) | What contracts am I bound by? |

**Review note:** the README sells the receipt as the basis for *"billing, assignment of liability."*
Until `wallet`/`agreement` exist, the receipt is a forensic tool, not a marketplace. **But this is
blocked on protocol** — flag it as a cross-repo dependency, not a CLI-only task. **Discussion: confirm
EasyNet-Ledger/Agreement is on the Axon roadmap before we spec the CLI side.**

---

## SECTION 6 — Mission / EAL (orchestration)

> **Ontology anchor (decided 2026-05-31) — Mission is a SCRIPT, not a workflow.**
> Per `EasyNet-Axon/document/concepts/CONCEPT_MODEL.md:59–68`: Mission is a **compile-time artifact** —
> EAL → Mission IR (a declarative DAG) → **expanded into one or more Invocations**. It is explicitly
> *not* a runtime first-class entity (CONCEPT_MODEL.md:4 supersedes prior drafts that treated it as
> one). "Mission state" is just the aggregate of its expanded Invocations.
>
> **Consequence — the only runtime-addressable object is the Invocation.** This is the line that keeps
> Axon from growing a second runtime entity:
>
> | Allowed (Invocation-addressed) | Forbidden (would require a Mission Runtime State Store) |
> |---|---|
> | `retry(invocation_id)` ✅ | `retry(step_id)` ❌ |
> | `cancel(invocation_id)` ✅ | `patch(step_state)` ❌ |
> | `watch(invocation_id)` ✅ | `resume(step_n)` ❌ |
> | `receipt(invocation_id)` ✅ | |
>
> A step-level retry forces the system to remember "which step, which failed, which retried" — i.e. a
> checkpoint/scheduler/step-transition store. The moment that store exists, Mission **is** a Workflow
> Instance and Axon has reinvented Temporal / Airflow / Dagster / Prefect. **We refuse that.** Retry is
> *re-invoke*: a new Invocation linked to the old one via `causal_context`/`causal_binding`
> (types.proto:343, invoke.proto:696) — `old invocation → new invocation`. Three-layer closure holds:
> **L0 EAL script → L1 Invocation → L2 Receipt**, with no fourth runtime entity.

> **The compiler analogy (the design constitution for EAL).** "Don't over-engineer Mission" does NOT
> mean "don't optimize." It means **optimize in the right layer.** EAL maps cleanly onto a compiler
> stack, and the codebase already reflects it (`src/eal/planner.rs` is an analyzer+lowering pass that
> does phase assignment, topological lowering, cycle/type checks, parallel-when-independent scheduling):
>
> | Compiler concept | EasyNet layer |
> |---|---|
> | source / IR | **EAL → Mission IR** |
> | compiler (planner / optimizer) | **`src/eal/planner.rs`** (analyze → assign phases → lower to IR) |
> | ISA + execution substrate | **Axon runtime** |
> | instruction | **Invocation** |
> | execution trace / commit log | **Receipt** |
>
> Because optimization lives in the **compiler/runtime** layers — not in a mutable Mission object —
> aggressive optimization is *encouraged*, and none of it grows a second runtime entity:
>
> | ✅ Allowed (compiler/runtime optimization) | ❌ Forbidden (mission grows a runtime body) |
> |---|---|
> | compile-time DAG rewrite | mission-owned mutable state |
> | cost-based routing | mission-level checkpoint |
> | operator fusion | mission-level control plane |
> | speculative execution | step object lifecycle |
> | memoization | |
> | parallel scheduling | |
> | retry-policy lowering (compile retries into the IR; runtime executes them as re-invokes) | |
>
> **One line:** *EAL may be like a query plan; it must not be like a workflow engine.* Its value is
> turning agent execution into an **optimizable, auditable, rewritable Invocation program** — exactly
> the leverage a query optimizer gives SQL, with none of the stateful-orchestrator baggage.

| Command | Status | Pri | Notes |
|---|---|---|---|
| `mission compile <FILE> [--emit-ir]` | SHIPPED | — | EAL → Mission IR. Pure compile step. |
| `mission run <FILE> [--trace]` | SHIPPED | — | Compile + **expand IR into Invocations**. Produces a `root_invocation` with children. |
| `mission list [--limit] [--format]` | SHIPPED | — | Lists past runs (= past root invocations). |
| `mission show <ID> [--trace]` | SHIPPED | — | |
| `mission discuss <AGENTS>... [--prompt] [--rounds]` | SHIPPED | — | |
| `mission think [--worker] [--judge] [--curator] [--cycles]` | SHIPPED | — | |

### Operate on the Invocation, not the Mission (the runtime verbs live here)

> `mission run` is the only "mission" verb at runtime — it *launches*. Everything after launch addresses
> the **Invocation tree**, because that's the only thing that exists at runtime.

| Command | Status | Pri | Backing | What it's for |
|---|---|---|---|---|
| `invocation watch <id> [--tui]` | **MISSING** | **P1** | proto-rpc (`InvokeStream` invoke.proto:93) | Stream a root invocation's causal subtree live; `--tui` = the tracking UI (§6.1). The only window that *shows* the protocol's value (signed receipts + IR phases + scheduling). |
| `invocation cancel <id>` | PARTIAL (as `mission cancel`) | P1 | proto-ability (`meta.cancel`) | Cancel the root invocation; cancellation propagates down the causal subtree. Today spelled `mission cancel`; should generalize to any invocation id. |
| `invocation retry <id>` | **MISSING** | P1 | proto-ability (`meta.acquire`/re-invoke; linked via `causal_context` types.proto:343) | **Re-invoke**: spawn a *new* Invocation from a failed one, linked by `receipt.causal_parent`. NOT step-retry. |
| `invocation receipt <id>` | SHIPPED (as `invocation show/trace`) | — | proto (`InvocationReceipt` invoke.proto:659) | Already covered in §8. |

### ❌ Deleted — workflow-model verbs that would reintroduce a second runtime entity

> These were in an earlier draft. **Removed on ontology grounds**, not complexity grounds.

| Removed command | Why it's forbidden |
|---|---|
| ~~`mission retry-step <ID> <STEP>`~~ | Requires addressable, stateful steps → Mission Runtime State Store → Workflow Instance. Use `invocation retry <child-id>` instead. |
| ~~`mission resume-step`~~ / ~~`mission patch-step`~~ | Implies checkpoint + step-transition state. Same regression. |
| ~~`mission abort <ID>`~~ | Folds into `invocation cancel <root-id>` (hard vs graceful is a flag on the invocation, not a separate mission control plane). |
| ~~`mission timeline <ID>`~~ | Not a workflow execution graph — it's the **causal tree of invocations**, rendered by `invocation watch --tui`. |

> **⚠️ Conflict with current RFC-001 mapping:** the restatement doc (mapping §, line 262) currently
> restates `RetryStep` → ability **`mission.retry_step`** with a `{mission_id, step_id}` signature —
> i.e. it bakes the workflow model into the protocol. **This review formally objects:** that ability
> should be dropped in favor of re-invoke. Flag for the Axon team to amend the mapping.

**Review note:** this domain is the strongest differentiator and already good — *because* Mission is a
script (a compiler front-end for `invoke`), not a workflow engine. Operability under failure is handled
at the **Invocation** layer (`invocation retry/cancel/watch`), not by a mission control plane. That
distinction is what keeps Axon's runtime to a single primitive and out of the Temporal/Airflow lane.

---

## SECTION 6.1 — Invocation Causal-Tree TUI (master-detail) — P1

> **Renders an Invocation Causal Tree, NOT a Workflow Execution Graph.** This is the visual consequence
> of §6's ontology: there is no workflow object to monitor — there is a `root_invocation` and its
> causal subtree, each node a child Invocation with its own signed Receipt. Spelled `invocation watch
> <root-id> --tui`. A full-screen, three-pane, live view.
>
> **Every visible element maps to an existing protocol *message* — no new wire schema needed to
> render.** Data arrives via `InvokeStream` (invoke.proto:93), the surviving RPC. The left "Phases"
> pane is the EAL→IR phase partition (compile-time), the middle rows are **child invocations** (not
> workflow steps), the right pane is one **Receipt**. The one schema change that *should* happen is
> small and important
> (cost/usage into the receipt — see §6.2).

### Why this is P1, not P2 (product argument)

It is the **only window that makes the protocol's value visible.** A bare agent framework's detail
panel is `console.log` prettified; **EasyNet's detail panel is a rendering of an `InvocationReceipt`** —
callee-signed (`callee_signature`, invoke.proto:700), hash-chained (`self_hash`/`prev_receipt_hash`),
independently auditable offline. One screen lights up **four value axes at once**: ACCOUNT (signed
receipt chain), ORGANIZE (the Phases tree = EAL phase projection at runtime), MANAGE/USE
(pause/stop/save a long cross-machine run). Without it, those advantages are words in a doc; with it, a
user sees in one glance *why EasyNet, not a bare agent script.*

### Layout (merged from both screenshots)

```
  easynet mission watch <id> --tui

  ┌─ Phases ──┬─ Child invocations ──────┬─ Detail = one InvocationReceipt ───────┐
  │ ✓Inventory│ ✓ cls:ura      18.6k 34s │  ✓ Completed · Opus 4.8 · 57s          │ state+timestamps
  │ ▶Classify │ ▶ cls:AgentId  42.1k 57s │  caller → callee → subject  (signed)   │ axiom 7-tuple binding
  │  Synth..  │   cls:NodeId   …         │  cost: 42.1k tok / $0.0x   ◀ §6.2 gap  │ ECONOMIC (receipt field TBD)
  └───────────┤   …                      │  policy: allowed by rule X ◀ §4.5      │ PROTECT (policy why)
   39/72 ·9m  │  1–43 of 68 ↓            │  Activity: 6 tool calls (each=receipt) │ ACCOUNT (progress receipts)
              │                          │  Outcome: Verdict NEEDS-HUMAN-DECISION │ terminal payload+reason
   ↑↓ select · x stop · p pause · s save └────────────────────────────────────────┘
```

### Field mapping — every UI element → protocol field (verified)

| UI element | Backed by | Status |
|---|---|---|
| `39/72 agents · 9m12s` header | `MissionSummary.completed_steps/total_steps` + `total_elapsed_ms` (mission.proto:269-277) | ✅ |
| Left **Phases** tree (Inventory/Classify/Synthesize) | EAL phase partitioning (`planner.rs`, already computes `phase_of`) | ✅ EasyNet-unique |
| Child-invocation rows + ✓/▶/○ status dots | each row is a **child Invocation**, dot = its `InvocationState`; the IR's `MissionStep` is the *compile-time* origin, but at runtime the addressable thing is the invocation (state enum at types.proto:871) | ✅ |
| `Opus 4.8 (1M context)` per row | callee agent/model (`MissionStep.target_node` + agent registry) | ✅ |
| `1–43 of 68 ↓` pagination | `ListMissions` keyset cursor (mission.proto:262) | ✅ |
| Live refresh (✓ lighting up) | `WatchMission` stream → `MissionEvent` (mission.proto:289) | ✅ |
| Detail `✓ Completed · 57s` | receipt `state` + `timestamp` deltas (admitted→terminal) | ✅ |
| Detail `caller → callee → subject` | receipt axiom binding `caller_binding`/`callee_binding`/`subject_binding` (invoke.proto:692–694) | ✅ signed, offline-auditable |
| Detail `Activity · last 3 of 6 tool calls` | each tool call = one `progress` receipt (`receipt_type`, invoke.proto:663) | ✅ each signed |
| Detail `Prompt · 40 lines` / `Outcome` | receipt `payload` + `reason` (invoke.proto:676) | ✅ |
| Detail **`42.1k tok` (cost)** | **no token/cost field on the receipt** | 🔴 §6.2 |
| Detail **policy / permission** | policy/consent decision (§4.5 `policy why`) | 🟡 lives elsewhere, not in receipt |

### Build cost

- **Protocol:** 0 changes to render (everything above the two 🔴/🟡 rows already exists).
- **Aggregation:** small — bucket `MissionEvent`s by phase, accumulate token/tools/duration. Reuse
  `src/runtime/timeline.rs` skeleton.
- **Render:** medium — introduce `ratatui` + `crossterm` (CLI has **no TUI dep today** — this is the one
  architecture decision). Three-pane + footer.
- **Interaction:** small–medium — `↑↓` select · `x → CancelMission` · `s → save JSON`. **`p pause` has
  no mission-level proto primitive** (only Cancel/Abort exist) — either map to "stop dispatching new
  steps, let running ones finish" (small change) or drop the key for v1. **Open item.**

### Commands

| Command | Status | Pri | Backing | What it's for |
|---|---|---|---|---|
| `mission watch <id> --tui` | **MISSING** | P1 | proto (`WatchMission` + `ListMissions` + receipts) | Full-screen master-detail tracker; re-attach to any running mission. |
| `mission run <file> --watch` | **MISSING** | P1 | same | Inline tracker for a mission you just launched. |

---

## SECTION 6.2 — Protocol proposal: cost/usage into the receipt (the ACCOUNT↔ECONOMIC link)

> **Cross-repo item: EasyNet-Cli ↔ EasyNet-Axon.** This is the first concrete brick of the ECONOMIC
> axis (§5, today roadmap-only) — and the TUI (§6.1) is what surfaces the need.

**The gap:** verified — `InvocationReceipt` has **no token/cost/usage field**. (`types.proto`'s `cost`
is a 0.0–1.0 *scheduling weight*, line 999; `*_usage` is CPU/mem/GPU *utilization*, lines 625–627 —
neither is per-invocation token count or money spent.) So the `42.1k tok` in the screenshot is
**counted locally by the TUI from agent output — it never enters the signed receipt chain.** That means
it is **not auditable and not billable.**

**The proposal:** add a signed `usage` (and optional `cost`) field to `InvocationReceipt` — tokens,
duration, external-API spend — so "how much this step cost" is fixed and signed exactly like "who
called whom, doing what" already is.

**Why it matters (connects two axes):**
- **ACCOUNT** today records *what happened* but not *what it consumed* — half a ledger.
- **ECONOMIC** (wallet/billing, §5) is "air castles" without a *signed* usage record to bill from. You
  cannot trust a bill computed from unsigned local counters.
- Small effort: one field + populate it at receipt-emit time. Unlocks the whole §5 axis credibly.

| Item | Status | Pri | Backing | What it's for |
|---|---|---|---|---|
| `InvocationReceipt.usage` (+ optional `cost`) | **MISSING (proto change)** | P1 | proposal vs `InvocationReceipt` (invoke.proto:659) | Make per-invocation token/duration/spend a signed, auditable, billable fact. |

---

## SECTION 7 — Content / wedge products

| Command | Status | Pri | Notes |
|---|---|---|---|
| `skill install/list/upgrade/remove` | SHIPPED | — | Marketplace install. Good. |
| `pages create/list/show/delete/url` | SHIPPED | — | Folder→website (RFC-006-B). Smart wedge. |
| `api-key create/list/revoke` | SHIPPED | — | OpenAI-compat keys (RFC-006-C). |
| `llm-api <PROMPT> [--model] [--key] [--system] [--json]` | SHIPPED | — | One-shot OpenAI-compat completion. |
| `call create/show/join/leave/end/watch/metrics` | SHIPPED (CLI) | — | Voice/video. **But the backing is mid-restatement**: `Voice` service is collapsed (voice.proto:43); 14/16 voice + 5/7 stream RPCs restated as abilities, **2+2 still `NEEDS-DECISION`** in RFC-001. So the CLI verbs ship, but the protocol underneath is not finalized. |
| `skill install/list/upgrade/remove` | SHIPPED | — | Note asymmetry: pages/api-key/llm-api/call are self-serve GET *loops* (Route A); `skill install` is a one-way fetch with no matching publish loop. |

**Review note:** ~~no gaps flagged~~ **corrected:** the *CLI verbs* are complete, but (1) `call`'s
protocol backing has 4 open RFC-001 decisions — calling it "complete" overstated it; (2) voice/stream
as a whole has **no dedicated section** in this doc (see §10 coverage gaps). `call` is arguably ahead
of demand; fine to leave the CLI as-is, but track the open protocol decisions.

---

## SECTION 8 — Runtime, MCP, Federation, Observability

### Shipped
| Command | Status | Pri | Notes |
|---|---|---|---|
| `runtime start/stop/status/connect/logs` | SHIPPED | — | |
| `mcp serve/status/install/skill-install` | SHIPPED | — | |
| `federation peers/discover/gen-cert` | SHIPPED (read-only) | — | Look but can't act. |
| `invocation list/show/trace/path` | SHIPPED | — | The receipt surface. Strong. |

### Missing — OBSERVABILITY (P2, all proto-backed)
| Command | Status | Pri | Backing | What it's for |
|---|---|---|---|---|
| `health` | **MISSING** | P2 | proto (`GetNetworkHealth` observe.proto:59) | Network summary (nodes/abilities/invocations), not just local `doctor`. |
| `slo [--ability]` | **MISSING** | P2 | proto (`GetSLOStatus` observe.proto:131) | SLO compliance for abilities I serve. |
| `burn-rate` | **MISSING** | P2 | proto (`GetBurnRate` observe.proto:159) | Error-budget consumption. |
| `events --filter invoke.*` | **MISSING** (raw form in `auth events`) | P2 | proto (`WatchEvents` observe.proto:112) | Promote `auth events` to first-class, typed event filtering. |

### Missing — FEDERATION ACTIONS (P1)
| Command | Status | Pri | Backing | What it's for |
|---|---|---|---|---|
| `federation join <hub>` | **MISSING** | P1 | proto-ability (`federation.federate` ← federation.proto, Federation service collapsed at :51) | Join a *stranger's* hub. The transitional `JoinFederation` message exists but the RPC is collapsed; the CLI's `join` is device-pairing only and does **not** call this. |
| `federation nodes [--ability] [--tag]` | **MISSING** | P1 | proto (`ListFederatedNodes` federation.proto:292) | Underlies `discover`; also useful raw. |

**Review note:** `federation` is read-only today. The "join a stranger's hub → establish trust →
transact" journey isn't expressible. This pairs tightly with §3 (discover) and §4 (trust).

---

## SECTION 9 — Maintenance

| Command | Status | Pri | Notes |
|---|---|---|---|
| `self check/update/uninstall` | SHIPPED | — | |
| `doctor [--json]` | SHIPPED | — | Local health. Complements (not replaces) network `health` §8. |
| `completion <SHELL>` | SHIPPED | — | |

**Review note:** complete. No changes.

---

## SECTION 10 — Coverage gaps the audit surfaced (whole domains with no section)

> Found by the completeness critic comparing the doc's section coverage against all 13 proto files.
> These are domains the section-by-section walkthrough never reached.

| Domain | Proto | Status in doc | Verdict |
|---|---|---|---|
| **Voice / conference** | `voice.proto` (collapsed :43) | only 1 table row in §7, marked "complete" | **Under-covered + 4 open RFC-001 decisions.** Real-time multi-agent is a flagship axis; deserves its own treatment or an explicit "deferred to RFC-001 restatement" ruling. |
| **Stream (PTY / media sessions)** | `stream.proto` (collapsed :20) | implied by `InvokeBidi` mentions; no section | Restated as abilities (5/7), 2 `NEEDS-DECISION`. The CLI has `device exec`/terminal but the streaming model isn't reviewed. |
| **State sync / presence** | `state_sync.proto` (collapsed :47) | **0 mentions** | Either out-of-CLI-scope (say so explicitly) or a genuine hole. 5 RPCs → abilities. |
| **Admin / disaster recovery** | `admin.proto` (collapsed :43) | incidental mention only | 6 ops (`admin.drain_node`/`quarantine_node`/`failover`/`snapshot`/`recover`/`reload_limits`) → abilities, no CLI section. Operationally relevant for fleet owners (MANAGE axis). |
| **Payload transfer** | `transfer.proto` (`PayloadTransfer`, **KEEP** :25) | referenced for `--with-assets` | The *only* surviving non-Invoke service. Worth one explicit row: it's how `teach --with-assets` (§2.5) moves bytes. |

**Decision needed:** for each — write a section, or issue an explicit "out of CLI scope" ruling. Don't
leave them silently uncovered (silent omission reads as "covered").

> **Note on line citations:** all proto file:line references in this doc were re-pinned against current
> `core/proto/axon/v1/` HEAD after the audit found the original draft was pinned to a pre-RFC-001
> revision (≈3-line drift across the board). Citations now point at the transitional *message* location;
> the canonical name is the **ability** per §-1.

---

## Priority rollup (what to build, in order)

**P0 — the product becomes itself:**
1. `trust show` / `trust set` — P0, but blocked on confirming/publishing callable `identity.get_trust` / `identity.set_trust`; messages alone are not enough.
2. **`policy` + `permission`** — the authorization control plane (§4.5). Co-equal with trust: protects service, data, resources. Messages exist, but callable `policy.*` / `capability.*` ability publication is the cross-repo dependency.
3. `discover "<intent>"` — the marketplace moment; `ListFederatedNodes` filtering ships today, semantic ranking is roadmap.
4. `wallet` / `agreement` — **blocked on Axon protocol** (Ledger + Agreement primitive). Flag as cross-repo dependency.

> **Why trust + permission are both P0:** trust = *"how much do I trust this stranger"*; permission =
> *"what is this caller allowed to do to my stuff."* You need both before you let strangers' agents
> touch your machine. They are the two halves of one control plane.

**P1 — credible for cross-owner publishing:**
4. `ability sign` / `promote` / `rollback` / `consent` — message-backed release pipeline subset; callable ability publication must be verified before implementation is scoped as CLI-only.
5. **The object-model project (§3.7 + §3.8)** — *not three commands, one coherent model.* In order: (a) implement `visibility`/`authorized_agents` so encapsulation is real (today it's an honor system); (b) parse the URA `family` segment into `ParsedURA`; (c) build a minimal `PolicyRule.condition` evaluator that can read `ability.family` + prefix-match (`aris.*`) — today admission is 2 hardcoded checks and `condition` is never read. Only *after* (b)+(c) does §3.7's "family-scoped policy for free" become true; today it's a prerequisite chain, not a freebie. Object model is for encapsulation/discovery/authorization — **never** addressing/routing.
6. **`ability teach` / `learn` (§2.5 + §3, GET route B)** — owner-driven capability transfer. `learn`/`forget` meta-semantics already exist (`meta.acquire`/`meta.forget`, ontology:220–221); the gap is the cross-owner `teach` half + the `InstallPolicy` safety gate (capability.proto:235–239). Depends on `visibility` (P1.5a) being real first.
7. `agent register` / `resolve` — agent-as-network-citizen.
8. `federation join <hub>` / `federation nodes` — make federation actionable.
9. Promote `whoami` to top-level.
10. **`mission watch --tui` + `run --watch` (§6.1)** — the master-detail tracking UI. The only window that *shows* the protocol's value (signed receipts + phase tree + distributed scheduling). All data exists; needs a `ratatui` render layer. Lights up ACCOUNT+ORGANIZE+MANAGE+USE at once.
11. **`InvocationReceipt.usage`/`cost` (§6.2) — cross-repo proto change.** First brick of ECONOMIC: make per-invocation token/duration/spend a *signed* fact, not a local TUI counter. Small effort, unblocks §5 credibly.

**P2 — operability & polish:**
12. `ability from-cli` / `from-openapi` / `from-mcp` (§3.5) — first-class front door for adapters that already ship.
13. `ability study` (read-only learn) / `forget` (unlearn) — round out GET route B.
14. `invocation retry` / `cancel` (generalize from `mission cancel`) — failure-mode operability at the **Invocation** layer (§6). NOT `mission retry-step` (deleted on ontology grounds — would reintroduce a workflow state store).
15. `health` / `slo` / `burn-rate` / `events`.
16. Naming cleanup: `auth` → identity-only; finish `agent discuss` → `mission discuss`.

---

## Open questions for the team (decide before speccing)

1. **Permission groups (§4.5)** — ship `policy` + `permission` as two command groups, or unify under one `access` noun? Condition authoring: raw expression vs guided flags vs both?
2. **Agent identity** — is `RegisterAgent`/`ResolveAgent`/`MigrateAgent` a v1.1 Axon commitment? (decides §2 P1 vs P2)
3. **Economic layer** — is EasyNet-Ledger + Agreement primitive actually on the Axon roadmap with a target? (gates all of §5)
4. **Release pipeline** — full pipeline at the CLI, or curated subset (`sign`/`promote`/`rollback`)?
5. **Semantic discovery** — is the pgvector ranking layer in scope, or does `discover` ship as substring-filter-only first?
6. **Ability families (§3.6 + §3.7)** — *Decided:* implement the URA namespace segment as the family dimension (don't delete it). Remaining forks: (a) keep wire word "namespace" but call the concept "family" in UX, to avoid colliding with `ResourceNamespace`? (b) do user-owned abilities get an explicit family slot now, or is `agent` treated as the family for user-owned while the explicit slot stays hub-owned? Family remains discovery/authorization-only — never addressable or routable.
7. **Object model (§3.8)** — audit found namespace hierarchy/recursion, permission-binding, and OOP structure are all dangling (flat-by-design + decoupled + doc-only). Forks: (a) a **base-Agent contract** (guarantees `chat`/`describe`) or convention? (b) **polymorphism** via an interface/ability-spec contract, or duck-typed-by-name? (c) `condition` evaluator full expression language or tiny matcher (trust + family-prefix + node-label)? *I lean: enforce `visibility` first (make encapsulation real), keep namespace flat, tiny matcher, defer recursive-Agent/multi-level to v2 unless forced.*
8. **GET route B — teach/learn (§2.5)** — product route decision: support BOTH remote-invoke (default) AND owner-driven teach/learn. Forks: (a) does `teach` confer manifest-only, or `--with-assets` for files/models too? (b) is `study` (read-only contract, no runnable copy) worth shipping, or does `show` already cover the "understand without installing" need? (c) what `execution_mode` default for taught code — `sandbox_first` (safe) vs `docker_only` (isolated) vs `host` (trusted-only)? *Owner initiative + `allow_transferred_code=false` default are non-negotiable — capability is conferred, never pulled.*
9. **Mission TUI (§6.1)** — (a) accept introducing `ratatui`/`crossterm` as the CLI's first TUI dependency? (b) `p pause` — map to "stop dispatching new steps, let running finish" (needs a small proto/runtime change) or drop the key for v1 since only `Cancel`/`Abort` exist? (c) `mission watch --tui` and `run --watch` share one render core?
10. **Receipt usage/cost (§6.2)** — cross-repo proto change to `InvocationReceipt`. (a) confirm Axon owners accept adding a signed `usage` (+ optional `cost`) field? (b) scope of `usage`: tokens + duration only, or also external-API spend / resource units? (c) is `cost` a money amount (needs currency + pricing source) or deferred until the ECONOMIC axis (§5) lands? *This is the first brick of §5 — without signed usage, billing has no trustworthy basis.*

---

*All `proto` backings verified against `core/proto/axon/v1/*.proto` on 2026-05-30.
All `roadmap` items confirmed absent from proto (need protocol work first).*
