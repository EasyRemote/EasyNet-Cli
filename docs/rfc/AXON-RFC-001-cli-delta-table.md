# AXON-RFC-001 — EasyNet-Cli Delta Table

> **Source of truth:** [`EasyNet-Axon/docs/rfc/AXON-RFC-001-invocation-as-sole-network-primitive.md`](https://github.com/EasyRemote/EasyNet-Axon/blob/main/docs/rfc/AXON-RFC-001-invocation-as-sole-network-primitive.md)
>
> This document is the per-repo Delta Table for EasyNet-Cli. It enumerates every file, symbol, and command in this repo that violates the RFC and what it must become. **No code changes in this PR** — only documentation.

| Field | Value |
|---|---|
| Status | Draft (open for review) |
| RFC | AXON-RFC-001 |
| Repo | EasyNet-Cli |
| Implementation phase | P2 (after axon P1 lands) |
| Backwards compatibility | NONE — hard break |

---

## §1  What gets renamed

The entire `system.*` ability namespace is retired. Every former system ability becomes an ability of a concrete Agent advertised through `federation.advertise_abilities`.

### `system.*` → namespaced ability under a concrete Agent

| Current name | New name | Owning Agent |
|---|---|---|
| `system.ping` | `meta.describe` (returns identity + ability summary) | Any Agent (universal reflexive ability) |
| `system.session.list` | `fleet.list_sessions` | device-agent |
| `system.session.attach` | `fleet.attach_session` | device-agent |
| `system.permission.subscribe` | `consent.subscribe` | consent-agent |
| `system.permission.decide` | `consent.decide` | consent-agent |
| `system.discuss.create` | `discuss.create` | device-agent (or dedicated discuss-agent) |
| `system.discuss.post` | `discuss.post` | same as above |
| `system.discuss.subscribe` | `discuss.subscribe` | same as above |
| `system.schedule.add` | `schedule.add` | device-agent (or schedule-agent) |
| `system.schedule.list` | `schedule.list` | same |
| `system.schedule.remove` | `schedule.remove` | same |
| `system.schedule.enable` | `schedule.enable` | same |
| `system.loop.create` | `loop.create` | device-agent (or loop-agent) |
| `system.loop.status` | `loop.status` | same |
| `system.loop.subscribe` | `loop.subscribe` | same |
| `system.loop.cancel` | `loop.cancel` | same |
| `system.skill.list` | `fleet.list_abilities` (with `visibility=PRIVATE` filter) | device-agent |
| `system.memory.list` | `meta.list_abilities` per sub-agent, or a `memory.*` ability on the relevant sub-agent | per-sub-agent |
| `<agent>.chat` (per-sub-agent chat) | `conversation.send` / `conversation.stream` | the LLM sub-agent |

### CLI command renames

| Current command | New command | Notes |
|---|---|---|
| `easynet skill list` | Stays. Internally uses Invoke against `fleet.list_abilities` instead of file-system walk. |
| `easynet ability invoke <name>` | Stays. Internally Invoke RPC. |
| `easynet ability list` | Stays. Internally Invoke against `meta.list_abilities` / `fleet.list_abilities`. |
| `easynet runtime start` | Stays. Boots the embedded axon kernel + advertises Agents per config. |
| `easynet runtime stop` | Stays. |
| `easynet agent add <name>` | Stays as a CLI verb. Internally Invoke against `fleet.start_agent`. |
| `easynet agent remove <name>` | Stays. Internally Invoke against `fleet.stop_agent`. |
| `easynet pair --as-hub` | Stays. Internally writes config + advertises an Agent with `federation.*` abilities. |
| `easynet join <token>` | Stays. Internally Invoke `federation.join` against the hub URA encoded in the token. |

CLI surface is **stable for users**. The hard break is wire-level only.

---

## §2  What gets deleted

### Files / modules

| Path | Action | RFC reference |
|---|---|---|
| `src/runtime/system/permission_ability.rs` | DELETE | §10 — replaced by consent-agent advertising `consent.*` |
| `src/runtime/publish.rs::publish_system_abilities_to_local_runtime` | DELETE | §10 — register-tool primitive eliminated |
| `src/runtime/publish.rs::publish_agent_to_local_runtime` | REWRITE — advertise abilities via `federation.advertise_abilities` Invoke instead of `register_runtime_local_mcp_tool` | §1, §10 |
| `src/registry/a2a_labels.rs::system_skills_json` (and the entire `system_skills[]` discovery payload) | DELETE | §10 — discovery is per-Agent ability advertisement, not a global label |
| `src/registry/a2a_labels.rs::description_for` (the `system.*` match) | DELETE | follows from §10 |
| `src/runtime/system/mod.rs::published_abilities` / `description_for` / `input_schema_for` | DELETE | §10 — system abilities don't exist |
| `src/runtime/system/skill_ability.rs` | DELETE — replaced by device-agent's `fleet.list_abilities` ability handler | §10 |
| `src/facade/cli/start.rs::republish_system_abilities_best_effort` | DELETE | §10 — system abilities don't exist |
| `src/facade/cli/start.rs::republish_all_agents_best_effort` | REWRITE — daemon advertises Agents via `federation.advertise_agent` + `federation.advertise_abilities` instead of `register_runtime_local_mcp_tool` | §10 |

### Strings that MUST disappear from `src/`

A CI boundary script will enforce: after P2 lands, the following strings must not appear anywhere in `src/`:

```
"system."                               (the system.* ability namespace)
register_runtime_local_mcp_tool         (the half-built primitive)
unregister_runtime_local_mcp_tool       (same)
runtime_local_tools                     (the parallel catalog)
system_skills_json                      (the discovery label)
publish_system_abilities                (the workaround we added today)
```

---

## §3  What gets added

### New Agent role implementations (each is just a struct + ability handlers, no new types)

| Agent (informal name) | Advertises | Implementation file |
|---|---|---|
| **device-agent** (default-on, one per daemon) | `fleet.*`, `meta.*`, `discuss.*`, `schedule.*`, `loop.*` | `src/runtime/agents/device.rs` (NEW) |
| **consent-agent** (default-on, replaces permission broker) | `consent.*` | `src/runtime/agents/consent.rs` (NEW) |
| **policy-agent** (opt-in, was permission policy logic) | `policy.*` | `src/runtime/agents/policy.rs` (NEW) |
| **mcp-bridge-agent** (opt-in, runs MCP server for external clients) | `mcp.bridge.*` | `src/runtime/agents/mcp_bridge.rs` (NEW) |
| **mcp-client-agent** (opt-in, dials external MCP servers) | `mcp.client.*` | `src/runtime/agents/mcp_client.rs` (NEW) |
| **hub-agent** (opt-in, this daemon serves as realm directory) | `federation.*` (+ optional `transport.relay.*`) | `src/runtime/agents/hub.rs` (NEW) |
| **claude / codex / etc. sub-agents** (one per registered AI) | `conversation.*`, `session.*`, `meta.*` | `src/runtime/agents/llm.rs` (NEW, replaces current chat handler) |

Each "Agent" above is just a struct that implements an ability handler trait. They are not protocol-level types — at the wire they are all `Agent { uri, identity, abilities[], metadata }` records.

### Embedded axon kernel

| Component | Source | Action |
|---|---|---|
| axon kernel (Invoke dispatcher, admission gate, causal context validator, receipt signer/store) | `EasyNet-Axon` crate | EMBED as a library dependency in `easynet-daemon` |
| `axon-runtime` standalone process | external binary, currently launched alongside daemon | DELETE — daemon hosts axon in-process |

---

## §4  Migration order within this repo (P2)

1. Add new ability namespaces alongside old `system.*` (so tests can run on both)
2. Add device-agent / consent-agent / policy-agent / llm-sub-agent implementations
3. Embed axon kernel; remove `axon-runtime` IPC dependency
4. Rewrite `publish.rs` to advertise via Invoke; delete `register_runtime_local_mcp_tool` calls
5. Delete `system.*` namespace; delete permission-broker side-channel
6. Update CI boundary script; verify zero forbidden strings in `src/`
7. Update `easynet ability list` / `easynet skill list` / `easynet agent *` to issue Invoke calls
8. End-to-end smoke against EasyNet backend (P3 must be ready)

---

## §5  Reviewer notes

- This PR adds **only** the Delta Table document. No code, no tests, no behavior change.
- Implementation will land in subsequent PRs; this document is the binding scope contract.
- Any item in §1 or §2 that is incorrect or missing should be raised on this PR before P2 starts — adding/removing items mid-implementation is expensive.
- The RFC's §10 lists this repo's deletions in summary form; this document is the file-level expansion.
