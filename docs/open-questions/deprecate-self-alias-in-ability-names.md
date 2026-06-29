# Open Question — Deprecating the `legacy self alias` Ability-Name Alias

**Status:** Open · **Trigger-based revisit** (not date-based) · **Owner:** Silan Hu · **Author:** 凉冰 (architect) · **Date:** 2026-05-05

## Summary

The literal `legacy self alias` token currently appears as a *namespace prefix* in EasyNet ability names — `<agent>.discover`, `<agent>.invoke`, `device.keyring.sign`, `session.open`, `identity.register_pubkey`, `legacy self alias.api_key.create`, and similar. This token is **late-binding sugar** that the dispatcher resolves at call time using the `caller` URA on the AXIOM Invocation envelope. It is *not* a URA, and it does not appear anywhere in the v4.1.5 §A.URA grammar.

Co-locating a dispatch-time variable in the ability-name namespace breaks several invariants that the rest of the system depends on. This document captures **why** the alias is wrong, **where** it leaks, and **the staged path** to remove it. It is filed as an open question rather than an executed PR because the change is protocol-level (device ⇄ hub wire), spans ~30 files / 75 active references, and must coordinate with EasyNet-Axon and the LLM prompt corpus.

---

## Why `legacy self alias` is the wrong abstraction

### 1. URAs are self-describing; `legacy self alias` is not

URA §A.URA-1 invariant: any URA, in isolation, names exactly one actor in the seven-tuple ontology. Given the bare string

```
easynet:///r/easynet.run/agent/test.claude
```

a reader can answer *who is this* without any additional context: realm `easynet.run`, user `test`, agent `claude`.

Given the bare string `legacy self alias`, a reader can answer *nothing*. The token is a parameter that gets bound at dispatch time against the caller's URA. The URA grammar has no slot for "depends on caller" precisely because every other AXIOM mechanism — visibility, signing, audit, federation routing — needs the seven-tuple to be a closed value, not a function of context.

`legacy self alias` therefore exists in a category of one: it is a *URA-shaped string* that is not a URA. The rest of the codebase has no way to tell those two cases apart at the type level, so every consumer either does ad-hoc `starts_with("legacy self alias.")` sniffing or silently misroutes the alias.

### 2. The same prefix is used for two different semantic layers

`legacy self alias.` is currently registered by two mechanisms with entirely different meanings:

**Layer A — per-agent self-alias.** Examples: `<agent>.discover`, `<agent>.invoke`, `legacy self alias.api_key.create`. Registered via `discover_ability::register_for_agent(reg, agent_name, …)` and friends. The dispatcher mounts one copy per LLM sub-agent (`claude`, `codex`, `web-builder`, …) and resolves `legacy self alias` against the caller's agent URA at dispatch time. The handler runs *as that agent*, the receipt's `callee` is that agent's URA, and visibility filtering scopes against that agent.

**Layer B — daemon device-bundle.** Examples: `device.keyring.sign`, `device.keyring.federate_user_identity_token`, `session.open`, `identity.register_pubkey`. Registered via `register_for_owner(reg, "legacy self alias", handle)` directly on the daemon, **without** a sub-agent context. The handler runs once on the daemon (it owns the device-scoped keyring or the long-lived bidi session to the hub); the `callee` is the device URA. Per RFC-002 §3.3 these abilities are *device-bundled* — their owner is conceptually the daemon, the `legacy self alias` prefix is purely historical.

The two layers share a token but have **disjoint owner kinds** (`agent/<u>.<a>` vs `device/<id>`) and **disjoint multiplicity** (one row per sub-agent vs. exactly one row). Telling them apart from the name alone requires knowing the registration site — which the descriptor catalogue does not preserve. As of this writing, `meta_ability::list_abilities_handler` synthesises both layers' descriptors with the device URA as owner, which is correct for Layer B by accident and wrong for Layer A.

### 3. Every downstream consumer has to re-implement de-aliasing

Search shows ~75 active references to `legacy self alias.` across the daemon, federation, keyring, advertise, CLI, and tests. Each call site that touches an ability name has had to decide independently:

- Does this string need `legacy self alias` resolution before I forward it?
- If I am persisting it (descriptor, receipt, audit log, federation advertise payload), do I substitute the resolved actor or keep the alias?
- If I am rendering it to a human / LLM / hub-side filter, which form do I show?

The answers diverge across modules. `runtime/agents/mod.rs:791,1285,1657,1777` filter on `name.starts_with("device.keyring.")` to special-case Layer B. `services/invocation_transport/admission_facade.rs:1710` hard-codes `session.open` as a dispatch target. `runtime/agents/discover_ability.rs:232` puts `<agent>.discover` into LLM-facing prompt copy. `services/invocation_transport/register_device_pubkey.rs:73` exposes `identity.register_pubkey` as a public constant on the wire. None of these sites can locally tell whether their fix is consistent with the others — every new `legacy self alias` reference adds an O(N) verification burden across the pre-existing N references.

### 4. The hub side speaks the alias too

`session.open` and `identity.register_pubkey` are not internal-only names: they are **on the wire** between device and hub. A device dials the hub with `function_name = "session.open"`; the hub-side admission gate accepts that exact string. Renaming on one side without coordinated rename on the other breaks pairing and severs the bidi session. Any clean-up plan must account for this rather than treating it as a single-repo refactor.

---

## What removal looks like (terminal state)

The terminal state encodes two principles:

**P1 — Owner is named at the registration site, not derived from the name.** The ability registry stores `(name, handler, owner_uri)`. `register_for_agent(reg, agent_name, …)` stamps the agent URA; `register_for_owner(reg, OwnerKind::Device, …)` stamps the device URA; `openai_compat_ability::register` stamps the realm hub URA. `meta_ability::list_abilities_handler` reads the stored owner verbatim, never sniffs prefixes.

**P2 — Ability names embed the owner explicitly.** Layer A becomes `<agent-name>.discover` / `<agent-name>.invoke` / `<agent-name>.api_key.create` (one row per sub-agent, name varies). Layer B uses first-class daemon namespaces: `device.keyring.sign`, `session.open`, `runtime.invoke_remote`, and `identity.register_pubkey`. The literal string `legacy self alias` does not appear in any descriptor, receipt, advertise payload, or wire field.

After P2, every ability name in the catalogue carries enough information to answer "whose verb is this" by inspection, and the `legacy self alias` resolution code path is deleted, not optional.

---

## Constraints on the migration

### C1 — Two-sided wire contract

`session.open` and `identity.register_pubkey` cross the device ⇄ hub boundary. A rename on the device side without a matching rename on the hub side breaks pairing for every paired device. The rename must therefore proceed in **double-name** mode for at least one release window:

1. Hub registers both `session.open` and `device.session` as the same handler.
2. Devices migrate to dialing `device.session`.
3. Once telemetry confirms no device dials `session.open` for one full release cycle, the hub drops the legacy alias.

The same pattern applies to `identity.register_pubkey` and any other on-wire `legacy self alias.*` name.

### C2 — LLM prompt corpus

Layer A names appear in LLM-facing system prompts and skill descriptions: `discover_ability::register_for_agent` writes copy that tells the LLM to call `<agent>.discover` to enumerate its own abilities. Renaming Layer A to `<agent-name>.<verb>` requires (a) updating the prompt template generator to substitute the concrete agent name, and (b) re-validating the LLM's discover-then-invoke flow under each supported model (RFC-006 adapter layer). This is the most reviewable but least automatable step.

### C3 — Persistent receipts and audit logs

Existing `InvocationReceipt` records on disk contain `function_name = "<agent>.discover"` etc. The migration must not invalidate or rewrite historical receipts: the receipt store is append-only and the alias is part of the historical wire fact. Audit-trail tooling (the receipt-link doc in `open-questions/axon-invocation-receipt-link.md`) needs to read both the legacy and post-migration form.

### C4 — Federation directory and advertise payloads

`federation.advertise_abilities` ships descriptors out to the realm hub, and the hub's directory keys agents by descriptor names. If a device advertises `<agent>.discover` for `claude`, the hub directory has one entry; if the same device renamed to `claude.discover` advertises again, the hub gets a *new* entry without retiring the old one — directory drift. The advertise path must coordinate the rename with a `federation.unpublish_ability` for the old name in the same publish-pass.

---

## Migration plan (staged)

### Stage 0 — Establish the registry-side invariant (no behaviour change)

**Goal:** every ability registration declares an explicit owner URA at the call site. The descriptor synth path stops sniffing names.

**Scope:** ~15 register call sites in `runtime/agents/` and `runtime/keyring/`. No wire changes, no name changes.

**Ship criteria:**
1. Daemon Axon catalogue registration APIs (rpc/bidi/stream/envelope variants) accept an `OwnerKind` parameter or pre-resolved `owner_uri`.
2. `meta_ability::list_abilities_handler` reads owner from the registry, deletes the `name.starts_with("01HUB.")` branch and the `device_owner` fallback for `legacy self alias.*`.
3. CLI render layer (`facade/cli/abilities.rs`) is unchanged because its input is already correct owner URAs.
4. New regression test: every registered ability name resolves to a parseable v4.1.5 URA via the registry's owner table.

This stage closes the open hole in this PR's design — that owner resolution happens by sniffing — without touching the alias itself. It is the prerequisite for stages 1 and 2.

### Stage 1 — Rename Layer A (per-agent self-alias)

**Goal:** Layer A ability names become `<agent-name>.<verb>` (e.g. `claude.discover`, `codex.invoke`). The `legacy self alias` prefix in Layer A is deleted.

**Scope:**
- `discover_ability::register_for_agent`, `invoke_ability::register_for_agent`, `api_key_ability::register_for_agent`, and any other per-agent registrar.
- LLM prompt template generators in `runtime/agents/profiles/llm/` and the agent-side skill copy.
- `runtime/workspace.rs:1018-1019` — the workspace ability-spec catalogue.

**Ship criteria:**
1. Each per-agent registrar mounts under the agent's own name. The prompt template substitutes that name.
2. LLM smoke test (`cargo test --features axon-pb -- discover_then_invoke_e2e`) passes for each supported sub-agent type.
3. Receipts written after this stage carry the new name; no double-write.
4. No `"legacy self alias"` literal in any active code path under `runtime/agents/`, except B-layer references.

**Wire impact:** none — Layer A is dispatcher-internal.

### Stage 2 — Rename Layer B (device-bundle), no compatibility aliases

**Goal:** Layer B ability names use their final daemon namespaces directly:
`device.keyring.*`, `session.open`, `runtime.invoke_remote`, and
`identity.{register_pubkey,list_user_pubkeys,revoke_user_pubkey}`. No old+new
dual registration window.

**Scope:**
- `runtime/agents/mod.rs:571` (keyring registration), all keyring abilities, `session.open`, `identity.register_pubkey`, `legacy self alias.api_key.*` (where it is in fact device-bundled), `legacy self alias.pages.*`.
- `services/invocation_transport/admission_facade.rs:1710`, `services/invocation_transport/register_device_pubkey.rs:73` and friends.
- `EasyNet-Axon` hub-side admission and bidi acceptors.
- `EasyNet` Go SDK if it carries any `legacy self alias.*` constants.

**Ship criteria:**
1. Hub/daemon registers only the final names.
2. Device, CLI, federation, and backend call only the final names.
3. No `legacy self alias.*` dials are accepted.
4. `services/invocation_transport/register_device_pubkey.rs` exposes only `ABILITY_IDENTITY_REGISTER_PUBKEY`.

**Wire impact:** breaking for any third-party device speaking `session.open` raw. Documented in the AXON-RFC-001 v4.1.6 changelog (this RFC bump becomes the carrier).

### Stage 3 — Cleanup

1. Delete every `name.starts_with("legacy self alias.")` filter and every `legacy self alias` literal from `src/`.
2. Add a lint (cargo-deny or a custom `cargo xtask check-uri-shapes`) that fails the build if `legacy self alias` reappears in an ability name string.
3. Receipt / audit reader retains the ability to *display* legacy `legacy self alias.*` names from historical receipts (read-only; do not normalise the historical record).
4. Mark this open-question doc as **Closed**, link the RFC bump and the cross-repo PRs.

### Stage 4 — System namespace partitioning (the deeper structural fix)

**Goal:** every system / built-in verb on the daemon lives under the canonical owner-prefix `device.<namespace>.<verb>` or `hub.<namespace>.<verb>`. The flat namespace under which `fs.read`, `fleet.list_nodes`, `voice.create_call`, `01HUB.openai.chat_completions` etc. live today is retired. After this stage the only first-segments in any catalogue ability name are:

* `device.*` — every device-internal verb (the current keyring registration shape, generalised to all 24 system namespaces).
* `hub.*` — every hub-tier verb (RFC-006-C OpenAI compat surface, future hub-rooted families).
* `<agent>.*` — verbs owned by a sub-agent (`codex.chat`, `web-builder.todo_add_task`, …).
* `<user>.*` — verbs owned by a user (`<user>.pages.*`, `<user>.files.*`, `<user>.api_key.*`).

The current `session_initiator.rs::SYSTEM_NAMESPACES` skip list (24 entries) collapses to a structural test:

```rust
if owner == "device" || owner == "hub" { continue; }
```

**Why this stage is necessary, in addition to Stages 1–3.**
The flat namespace conflates "namespace" with "agent identity". The session-prelude algorithm, the federation directory's agent enumeration, and every `easynet ability list` consumer that splits names on `.` to derive an owner, all see `fs` / `fleet` / `voice` first segments and have no structural way to tell those apart from a real sub-agent name like `codex`. The `SYSTEM_NAMESPACES` skip list shipped on 2026-05-05 is a hot-fix that papers over the conflation without resolving it: every newly-added system namespace must remember to update the skip list, and any consumer that does not consult the list ships the same 29-fake-agents bug we have already paid for once on the Frontend Agents page.

Stage 4 makes the conflation impossible: `fleet.list_nodes` becomes `device.fleet.list_nodes`, and the prelude's owner derivation is a single `if owner == "device"` check. New system verbs are mounted under `device.*` from day one and cannot regress.

**Scope.**

* **EasyNet-Cli (Rust device)** — ~95 ability names, ~50 `register_rpc(...)` call sites, ~1170 string occurrences across ~96 files (per the 2026-05-05 audit). 24 namespaces affected: `fs`, `http`, `shell`, `process`, `fleet`, `observe`, `admin`, `easynet`, `meta`, `mission`, `schedule`, `loop`, `discuss`, `mcp`, `a2a`, `policy`, `consent`, `camera`, `mic`, `screen`, `speaker`, `voice`, `skill`, `ability`. `01HUB.*` migrates to `hub.*` in lockstep.
* **EasyNet/backend (Go hub-mode)** — `daemonInternalAbility()` already accepts `device.*` (Step A). The other 23 system namespaces have ~44 affected files; the routing classifier needs to accept the new shape, and any backend handler that addresses a daemon ability by name (terminal handler, openai files handler, fleet operational endpoints) needs to dial the new name.
* **EasyNet-Axon (Go axon hub)** — wire dispatch table for hub-side verbs (`hub.openai.*`); admission accepts new names alongside legacy.
* **LLM prompt corpus** — `skills/easynet-collaborate/SKILL.md` and any other agent-facing prompt that hardcodes ability names.
* **TOML manifests under `abilities/system/`** — `gen-ability-tomls` regenerates against the new names; the old TOML files retire.
* **Receipt / audit reader** — historical receipts carry the legacy `function_name`; reader displays both forms.
* **Frontend** — any UI text or skill catalogue rendering that hardcodes a system ability name.

**Ship criteria.**

1. Every `register_rpc(name, ...)` site under `runtime/agents/` (and equivalents) takes a name with a `device.` or `hub.` prefix; no top-level system namespace remains.
2. EasyNet-Axon hub registers both legacy and new names as aliases of the same dispatch entry; one full release cycle of double-name acceptance before the legacy form is dropped.
3. Backend's `daemonInternalAbility()` simplifies to two arms (`device.*`, `hub.*`); the manual list under `SYSTEM_NAMESPACES` in `session_initiator.rs` is deleted.
4. LLM prompt corpus regenerated; smoke tests confirm `discover` → `invoke` flow for each sub-agent type.
5. `easynet ability list` shows the new shape; the truth-table spec (`docs/spec/owner-truth-table/`) is updated to match the terminal form.
6. Lint added: any new `register_rpc(name, ...)` whose name's first segment is not in `{device, hub, <agent-id>, <user-id>}` fails the build.

**Wire impact.** Breaking unless gated by Stages 1–2's wire-break window. This stage MUST share the AXON-RFC-001 v4.1.6 carrier with Stages 1–2; landing them separately produces partial-rename hub/device pairs that mis-route every system call.

---

## What to do in the meantime

Until Stage 0 lands:

1. **Do not add new `legacy self alias.*` ability names.** New device-bundle abilities are named `device.<verb>` from the start; new per-agent abilities are named `<agent>.<verb>`. The alias is closed for new contributions.
2. **Do not extend `legacy self alias` semantics across new dispatch paths.** Specifically, do not introduce code that resolves `legacy self alias` against a non-`caller` URA (e.g. against `subject` or against a delegation chain). Keeping the existing two-layer split frozen makes the eventual migration a pure rename.
3. **Render-layer treatment** (this PR's `facade/cli/abilities.rs`): the (DEVICE / AGENT / USER) projection of the `owner_agent_uri` is correct as-is; do not add `legacy self alias`-aware special-casing. When the registry-side invariant from Stage 0 lands, the rendering already aligns.
4. **Owner sniffing in `meta_ability::list_abilities_handler`** stays as currently written — synth uses `01HUB.` prefix sniff for hub URAs and `host_device_agent_uri` for everything else. This is not the terminal state but is the correct behaviour given the current registration API. Stage 0 deletes the sniff.
5. **`SYSTEM_NAMESPACES` skip list (`session_initiator.rs`) must be kept in sync with new system namespaces.** Adding a new system verb without updating the list re-introduces the 29-fake-agents bug on the Frontend Agents page. Stage 4 deletes the list entirely; until then, a code review checklist item enforces the sync.

---

## Hot-fixes already shipped

The following narrow, in-place fixes have landed on the path to the staged migration. They reduce live-fire damage from the unresolved `legacy self alias` / flat-namespace design without committing to any wire change.

### HF-1 — Keyring rename `device.keyring.*` → `device.keyring.*` (2026-05-05)

* EasyNet-Cli `runtime/agents/mod.rs` registers keyring under owner `"device"` instead of the legacy `"legacy self alias"` literal. 11 catalogue rows shift accordingly.
* All `name.starts_with("device.keyring.")` filters and the test-fixture `format!("device.keyring.{verb}")` updated.
* Doc / error-message references in `runtime/keyring/`, `runtime/agents/meta_ability.rs`, `services/invocation_transport/federation_wrappers.rs` updated.
* `services/invocation_transport/session_initiator.rs::SYSTEM_NAMESPACES` adds `"device"` to the skip set so the new namespace does not get advertised as an agent.
* EasyNet/backend no longer owns an ability-name classifier; it submits complete Invocations and lets the CLI daemon resolve locality.
* EasyNet/backend wire-pinned constants use `runtime.invoke_remote` only for explicit low-level bidi helper tests and `identity.register_pubkey` for trust seeding.

Wire impact: breaking for legacy callers by design. Keyring and identity trust writes are daemon-internal; backend routing is pinned by service-context tests and daemon catch-all tests, not by a backend classifier.

### HF-2 — Frontend Agents page restored (2026-05-05)

The session-prelude algorithm in `services/invocation_transport/session_initiator.rs` derives "agents this device hosts" by splitting `ability_catalog` entries on the first `.` and taking the first segment as the agent name. This conflates system namespaces (`fs`, `fleet`, `voice`, …) with real sub-agent identities. Without filtering, the device advertises ~29 fake agents to the hub on every session reconnect, displacing real sub-agents from the Frontend Agents page.

Fix: explicit `SYSTEM_NAMESPACES` constant listing all 24 system namespaces plus `01HUB`, `device`, `legacy self alias`. The prelude scanner skips any entry whose first segment is in this set. Number of agents advertised drops 29 → 4 (the real `codex`, `web-builder`, plus synthesised `pages` / `files`).

Wire impact: zero. The change affects what the device chooses to advertise, not the wire dispatch table.

### HF-3 — `EASYNET_PAGES_USER` placeholder default (2026-05-05)

`runtime/agents/mod.rs` defaulted `EASYNET_PAGES_USER` to the literal string `"self"` when unset, producing misleading ability names like `self.api_key.create` even on production-paired daemons. Resolution order is now:

1. `EASYNET_PAGES_USER` env (operator override / docker e2e harness)
2. `credentials.json::username` (production path post-pairing)
3. Literal `"self"` (last-resort fallback for unpaired / pre-bootstrap state)

Wire impact: zero. The user-segment was already part of the locally-registered ability name; this fix just makes it reflect the canonical user-id.

### Why these are hot-fixes, not Stage 0

HF-1 / HF-2 / HF-3 land before Stage 0 because production breakage forced their hand:

* HF-1 produced HF-2 (the keyring rename's `device.*` namespace was advertised to the hub as a fake agent until the skip list was updated).
* HF-2 was a P0 user-visible regression on the Frontend Agents page.
* HF-3 was a long-standing hazard called out in the truth-table spec but not actioned until HF-2's debugging surfaced live `self.api_key.*` rows.

None of these substitute for the staged migration. Stage 0 still needs to ship the registry-side `OwnerKind` invariant; Stages 1–2 still need wire coordination; Stage 4 still needs to retire the flat namespace. The hot-fixes buy time for the RFC.

---

## Why this is filed here, not as a PR

The change spans:

- ~15 `legacy self alias.*` registration call sites (Stage 1 + Stage 2)
- ~75 active `legacy self alias.*` references in `src/` (Stage 1 + Stage 2)
- ~95 system ability names, ~50 register sites, ~1170 string occurrences across ~96 files for Stage 4 (system namespace partitioning)
- ~44 affected backend files for Stage 4
- LLM prompt corpus and skill copy
- Hub-side wire admission (EasyNet-Axon, two repos coordinated)
- Persistent receipt / audit reader compatibility (legacy + new names)
- TOML manifests under `abilities/system/` (regenerated)
- Frontend hardcoded ability-name references
- Cross-repo coordination with at least one release-window double-write

That is not one PR's worth of work, and it is not work that can land without an architecture-level decision on Stage 2's wire-break window. Filing it as an open question makes the rationale visible to anyone who reaches for `legacy self alias.foo` for a new ability and lets the eventual implementer cite this doc rather than re-derive the analysis.

The trigger to revisit is not a date but a decision: when Silan ratifies the migration window for AXON-RFC-001 v4.1.6 (the carrier RFC for the protocol-level rename), this doc moves from `open-questions/` to `decisions/` and the staged plan becomes a sequence of PRs.

---

## Log

| Date       | Event                                                              |
|------------|--------------------------------------------------------------------|
| 2026-05-05 | Filed by 凉冰 during the `easynet ability list` rendering audit. The audit established that the descriptor synth in `meta_ability` was stamping `legacy self alias.*` abilities with the device URA — coincidentally correct for Layer B (RFC-002 keyring), structurally wrong for Layer A (per-agent self-alias). The proper fix is removing `legacy self alias` from the namespace, not patching synth. |
| 2026-05-05 | HF-1 shipped: keyring renamed `device.keyring.*` → `device.keyring.*` (EasyNet-Cli) plus matching `device.*` routing arm in EasyNet/backend. Wire-pinned `session.open` / `runtime.invoke_remote` / `identity.register_pubkey` constants flagged with TODO comments pointing at this doc. |
| 2026-05-05 | HF-2 shipped: `session_initiator.rs::SYSTEM_NAMESPACES` skip set added after HF-1 caused the Frontend Agents page to display only fake system-namespace agents. Drop from 29 → 4 advertised agents (real `codex`, `web-builder` + synthesised `pages`, `files`). |
| 2026-05-05 | HF-3 shipped: `EASYNET_PAGES_USER` default changed from literal `"self"` to `credentials.username` fallback; `self.api_key.*` rows now correctly emit as `<canonical-user>.api_key.*` on paired daemons. |
| 2026-05-05 | Stage 4 added to the migration plan after HF-2 surfaced the deeper issue: namespace conflation, not just the `legacy self alias` token, is the structural problem. The flat namespace must partition under `device.*` / `hub.*` for the prelude algorithm to use a structural test instead of a skip list. |
| —          | Revisit: when Silan ratifies the RFC-001 v4.1.6 migration window covering the wire-break (Stages 2 + 4 share the carrier). Trigger-based, no calendar date. |
