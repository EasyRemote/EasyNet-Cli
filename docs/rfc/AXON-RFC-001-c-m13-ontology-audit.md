# C-M13 — Ontology audit: §18 standard ability registry vs. live CLI registry

Status: snapshot taken 2026-04-27 against `rfc-001-impl` HEAD (commit
`7a76cf8`, after C-M9a / C-M9a-iii / C-M10).

## Method

Cross-checked the binding §18 table in
`docs/rfc/AXON-RFC-001-plan-v4.1.2.md` against the live `LocalAbilityRegistry`
built by `runtime::agents::build_registry_with_services`. Source of truth for
the live side: the `pub const ABILITY_*` declarations + `register_*` calls
in `src/runtime/agents/`.

## Coverage by profile

### device-profile

| §18 ability                | Live? | Notes |
|----------------------------|-------|-------|
| `observe.health`           | ✅    | `ping::register` |
| `observe.network_health`   | ❌    | Not yet wired. Lift from existing health probes. |
| `fleet.list_sessions`      | ✅    | `session_ability::register` |
| `fleet.attach_session`     | ✅    | `session_ability::register` (stream) |
| `fleet.session_input`      | ❌    | Tracked C-M3b/c (PTY abilities) — bidi candidate. |
| `fleet.session_read`       | ❌    | Tracked C-M3b/c |
| `fleet.session_resize`     | ❌    | Tracked C-M3b/c |
| `fleet.session_close`      | ❌    | Tracked C-M3b/c |
| `fleet.list_agents`        | ❌    | Surface `AgentRegistry` snapshot — analogue of `a2a.bridge.list_skills`. |
| `fleet.start_agent`        | ❌    | Lift from `easynet agent add` plumbing. |
| `fleet.stop_agent`         | ❌    | Lift from `easynet agent remove` plumbing. |
| `fleet.list_abilities`     | ✅    | `skill_ability::register` |
| `fleet.skill_install`      | ✅    | `skill_install_ability::register` |
| `fleet.skill_remove`       | ✅    | `skill_install_ability::register` |
| `fleet.skill_upgrade`      | ✅    | `skill_install_ability::register` |
| `admin.failover`           | ❌    | No implementation yet (multi-host concern). |
| `admin.snapshot`           | ❌    | No implementation yet. |
| `admin.recover`            | ❌    | No implementation yet. |
| `admin.status`             | ❌    | Lift from existing daemon status probe. |
| `meta.describe`            | ❌    | Surface `LocalAgentCatalog` entry for self. |
| `meta.list_abilities`      | ❌    | Same payload shape as `mcp.bridge.list_tools` minus the MCP wrapper. |
| `meta.acquire`             | ❌    | "skill install" verb in §18 maps onto this; we currently expose it as `fleet.skill_install`. Audit: keep both names or alias. |
| `meta.forget`              | ❌    | Same as above for `fleet.skill_remove`. |
| `meta.publish`             | ❌    | Visibility-promotion verb — no impl yet. |
| `meta.compose`             | ❌    | Skill composition — no impl yet. |
| `meta.cancel`              | ❌    | In-flight invocation cancel — needs invocation-handle table. |
| `schedule.add` / `.list` / `.remove` / `.enable` | ✅ ✅ ✅ ✅ | `schedule_ability::register` |
| `loop.create` / `.status` / `.subscribe` / `.cancel` | ✅ ✅ ✅ ✅ | `loop_ability::register` |

### consent-profile

| §18 ability         | Live? | Notes |
|---------------------|-------|-------|
| `consent.request`   | ❌    | Currently invoked from inside the admission gate as a sub-call; not yet a registered ability handler. Lift the existing `PermissionService::request` path. |
| `consent.subscribe` | ✅    | `permission_ability::register` (stream) |
| `consent.decide`    | ✅    | `permission_ability::register` |
| `consent.list_pending` | ❌ | Trivial wrapper over `PermissionService::list_pending`. |

### policy-profile

| §18 ability        | Live? | Notes |
|--------------------|-------|-------|
| `policy.evaluate`  | ❌    | The kernel's admission gate calls into a `PolicyEvaluator` directly today; no ability surface. Lift it. |
| `policy.simulate`  | ❌    | Trivial reuse of `policy.evaluate` with a no-side-effect mode. |
| `policy.publish`   | ❌    | No impl (admin verb). |
| `policy.list`      | ❌    | No impl (admin verb). |

### llm-profile

| §18 ability                          | Live?       | Notes |
|--------------------------------------|-------------|-------|
| `conversation.send` / `.stream`       | ⚠️ partial | Currently exposed as `<agent>.chat` per registered agent. §18 names them as `conversation.*` on the llm-profile Agent. **Decide:** rename `<agent>.chat` → `conversation.send` (with the agent identified by the callee URA, not the ability name)? |
| `session.create` / `.list` / `.resume` / `.close` | ❌ | Tracked C-M3b/c (PTY abilities are session-shaped); the §18 `session.*` namespace is the LLM-profile sibling. Need disambiguation from `fleet.*_session*` names. |

### mcp-profile

| §18 ability             | Live? | Notes |
|-------------------------|-------|-------|
| `mcp.bridge.list_tools` | ✅    | C-M9a |
| `mcp.bridge.call_tool`  | ❌    | Tracked C-M9a-ii (registry self-reference). |
| `mcp.client.list`       | ❌    | Tracked C-M9b (needs MCP client library). |
| `mcp.client.call`       | ❌    | Tracked C-M9b. |

### a2a (edge adapter) — additions beyond §18

| ability                  | Live? | Notes |
|--------------------------|-------|-------|
| `a2a.bridge.list_skills` | ✅    | C-M10 |
| `a2a.bridge.send_task`   | ❌    | Tracked C-M10-ii (same registry self-reference issue). |
| `a2a.client.send_task`   | ❌    | Tracked C-M10-iii. |

### Beyond §18 (live but not in the binding table)

| ability                                      | Provenance |
|----------------------------------------------|------------|
| `discuss.create` / `.post` / `.subscribe`    | Pre-RFC discuss feature; predates §18. **Decide:** add to §18 or sunset. |
| `<agent>.chat` per registered LLM agent      | Pre-RFC chat. See `conversation.*` row above. |

### federation / identity / transport — out of scope for CLI audit

These belong on hub-profile (Axon) and are tracked in the
EasyNet-Axon repo. C-M11 + C-M12 cover the federation event-stream
pieces that the CLI (as a client) consumes via Invoke.

## Decisions deferred to user

1. `<agent>.chat` vs `conversation.send` — keep both, alias, or rename?
   The §18 contract names the ability `conversation.send`; the live
   CLI uses `<agent>.chat`. Either rename live + provide one cycle of
   alias-shim, or amend §18 in plan v4.1.3.
2. `fleet.skill_install` vs `meta.acquire` — same shape, two names. Pick
   one or alias.
3. `discuss.*` — keep or sunset. They're popular in the existing CLI
   tests but absent from §18.

## Implementation ordering recommendation

Bucket A (cheap wins, no design questions):
- `fleet.list_agents` — analogue of `a2a.bridge.list_skills`
- `consent.list_pending` — wrapper over existing service
- `meta.describe` — wrapper over `LocalAgentCatalog`
- `meta.list_abilities` — already implicit via `fleet.list_abilities`,
  rename or alias

Bucket B (need fleshing-out, but not blocked):
- `fleet.start_agent` / `fleet.stop_agent` — lift from CLI plumbing
- `admin.status` — lift from daemon status probe
- `policy.evaluate` / `policy.simulate` — lift from `PolicyEvaluator`
- `observe.network_health` — lift from existing probes

Bucket C (blocked, tracked elsewhere):
- All `*.call_tool` / `*.send_task` — C-M9a-ii / C-M10-ii (registry self-ref)
- All `fleet.session_*` (PTY) — C-M3b/c (bidi infra)
- `mcp.client.*` — C-M9b (MCP client library)
- `meta.publish` / `meta.compose` / `meta.cancel` — need invocation-handle
  table + visibility-mutation primitives (own milestone, currently
  untracked)
- `admin.failover` / `admin.snapshot` / `admin.recover` — multi-host
  HA story not yet specified

## Tests

The existing
`runtime::agents::tests::published_ability_names_matches_live_registry`
+ `description_for_and_input_schema_for_cover_every_published_name`
already provide drift detection: any ability registered on the live
registry without a matching `description_for` / `input_schema_for`
arm fails CI. Adding a Bucket-A or Bucket-B ability requires updating
both arms, which keeps this audit table mechanically discoverable.

A future addition: a CI conformance test that lists the §18 binding
verbs and warns (not fails) on missing live entries. Failing would
block on Bucket-C indefinitely; warning surfaces drift without
blocking the train.
