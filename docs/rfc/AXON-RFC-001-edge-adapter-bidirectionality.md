# Edge adapter bidirectionality — MCP and A2A surfaces

## What this doc establishes

After C-M9a / C-M9a-ii / C-M9b / C-M10 / C-M10-ii / C-M10-iii, both
edge adapters (MCP, A2A) are **bidirectional first-class citizens**
of the ability surface. Outside callers can reach EasyNet through
either protocol; EasyNet can reach outside servers through either
protocol; and in both directions the surface looks identical to
the rest of the daemon: an Invoke-callable ability with a name, an
input schema, and a layer classification.

The "ability as MCP" property — that an ability is the same thing
whether it speaks the local Invoke ABI or a foreign edge protocol —
holds in both directions on one ability surface.

## The four-quadrant matrix

|              | Discovery (Introspection) | Dispatch (Operational)  |
|--------------|---------------------------|-------------------------|
| **Inbound MCP** (we are server)   | `mcp.bridge.list_tools` | `mcp.bridge.call_tool` |
| **Outbound MCP** (we are client)  | `mcp.client.list`       | `mcp.client.call`      |
| **Inbound A2A** (we are server)   | `a2a.bridge.list_skills` | `a2a.bridge.send_task` |
| **Outbound A2A** (we are client)  | (uses bridge.list_skills) | `a2a.client.send_task` |

Local Invoke is the fifth plane (canonical shape, full descriptor
catalogue) per `AXON-RFC-001-discovery-planes.md`.

## Symmetry invariants

For any ability X reachable through any plane:

1. **Same name across planes.** The byte-identical-name rule from
   `discovery-planes.md` applies. `meta.list_abilities` MUST list
   `mcp.client.call` as `mcp.client.call`; the MCP bridge MUST
   project it as `mcp.client.call`. No rename per plane.

2. **Bidirectional layering.** A `*.list` is Introspection
   regardless of direction (read-only, idempotent). A `*.call`
   or `*.send_task` is Operational regardless of direction (the
   side effects come from the dispatch target, not the
   bridge/client). The `classify_ability` table in
   `runtime/agents/mod.rs::tests` enforces this row by row.

3. **Failure shape parity.** Each surface returns structured
   `{isError: bool, content: [...]}` (MCP) or
   `{ok: bool, error?, result?}` (A2A) for handler-level failures.
   Wire-level failures (transport break, protocol violation) are
   the only ones that crash the connection. A stale catalogue
   client cannot bypass the visibility filter on either
   direction's call surface — both `mcp.bridge.call_tool` and
   `a2a.bridge.send_task` re-check against the live catalogue
   on each invocation.

## Why both directions matter

The half-implementation (only inbound) loses the most valuable
half of the property:

- **Inbound only**: external tools see EasyNet as an MCP server.
  Useful — Claude Code can install EasyNet and call its abilities.
  But EasyNet itself can't reach the operator's existing MCP fleet
  (filesystem MCP, context7, Linear MCP, …). They're invisible to
  any in-process caller, the planner, or another agent that wants
  to compose them.
- **Outbound only**: EasyNet can call external MCP servers. Useful
  — abilities can wrap upstream tools. But EasyNet's own abilities
  aren't reachable from anything that doesn't speak EasyNet's
  native protocol. The whole agent fleet stays an island.
- **Both**: any ability — local, MCP-bridged inbound, or
  MCP-mediated outbound — looks the same to the planner. A composed
  workflow can chain `meta.list_abilities` (catalogue read) →
  `mcp.client.call` (upstream filesystem) → `<agent>.chat` (local
  LLM) → `a2a.bridge.send_task` (federated peer) without the
  planner caring which plane each ability came from.

## Implementation notes

### Lazy connection lifecycle (mcp.client)

`McpClientService` spawns each upstream child process on first
use (mcp.client.list aggregates → first call fans out spawn per
configured server) and holds the connection for the daemon's
life. A future health-check could clear the connection on stdio
failure to trigger re-spawn; v1 surfaces the failure to the
caller and lets the next call retry.

### Visibility re-check on each call (mcp.bridge.call_tool, a2a.bridge.send_task)

Both bridge call surfaces fetch the live descriptor list and
re-verify the requested name BEFORE reaching the registry. A
stale list_tools client can't bypass a visibility filter that
changed between list and call. The MCP-shaped projection
(`tool_specs_from_descriptors`) is the source of truth for what's
callable.

### Registry self-reference via OnceLock seam

Both bridge call handlers need an `Arc<LocalAbilityRegistry>` to
dispatch into other local abilities. The chicken-and-egg —
registering a handler that needs a reference to the registry
being built — resolves via deferred initialisation: register
first, then `Arc::new(reg)` wraps it, then the build site sets
the OnceLock. Same shape `admin_status_ability` uses for its
ability-count provider. The lock is shared across both bridges
since they target the same registry; one allocation, two
consumers.

### Frame protocol reuse

Both inbound bridges take args in the format their respective
protocols expect (MCP: `{name, arguments}`; A2A: `{agent_name,
skill_name, args}`) and resolve them to the same in-process
registry name. For MCP that's the descriptor's `name` field
verbatim. For A2A it's `<agent_name>.<skill_name>` to match
how `chat_ability::register` installs `<agent>.chat` per loaded
agent. Both surface the resolved name in error messages so an
operator chasing a typo can grep for it.

## Out of scope (for now)

The following would extend the matrix but each needs a separate
design pass:

- **Streaming MCP / A2A**: MCP supports `notifications/progress`
  and `tools/sample`; A2A supports task events. Both map naturally
  to InvokeStream once the use case shows up. Not a v1 ability
  because no existing caller drives it.
- **Auth at the bridge boundary**: today both inbound surfaces
  trust the local socket's filesystem permissions for
  authentication. A federated A2A peer behind a remote-auth
  scheme would need DelegationProof verification at the bridge
  before reaching the call surface. Tracked as future C-M after
  federation matures.
- **mcp.bridge / a2a.bridge resource subscriptions**: MCP's
  `resources/list` and `resources/subscribe` are out of scope —
  EasyNet's discovery surface doesn't expose resources today.

## What this enables next

Bidirectional edge adapters are the foundation for:

- **Cross-protocol composition**: a planner can pipe MCP →
  EasyNet → A2A without the abilities knowing which protocol
  carried the call.
- **Federation with heterogeneous fleets**: an A2A peer running
  a non-EasyNet agent can be reached through `a2a.client.send_task`
  with no client-side library work.
- **Operator workflows on existing MCP investment**: an operator
  who's been managing MCP servers in `~/.claude/mcp_servers.json`
  can drop the same shape into `~/.easynet/mcp_clients.json` and
  every existing tool becomes Invoke-callable from anywhere
  EasyNet reaches.

That's the property the bidirectional matrix was for.
