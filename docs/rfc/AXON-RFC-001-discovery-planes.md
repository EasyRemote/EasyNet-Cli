# Discovery planes — unification

## The three planes

After C-M9a + C-M10 + C-M13-A4 there are now three abilities a
caller can use to ask "what does this device offer":

| Ability                  | Plane                  | Endpoint                  | Provenance                    |
|--------------------------|------------------------|---------------------------|-------------------------------|
| `meta.list_abilities`    | local Invoke           | `~/.easynet/control.sock` | live AbilityDescriptor catalog |
| `mcp.bridge.list_tools`  | MCP edge adapter       | `~/.easynet/mcp.sock`     | live AbilityDescriptor catalog (MCP-shaped projection) |
| `a2a.bridge.list_skills` | A2A edge adapter       | A2A discovery channel     | live AgentRegistry (A2A v2 envelope) |

These look like three surfaces. They are **one abstraction with
three projections**.

## Unifying abstraction: AbilityProvider

```
trait AbilityProvider {
    fn provenance() -> Provenance;        // local | mcp_bridge | a2a_bridge
    fn descriptor_shape() -> Shape;       // canonical | mcp_tool | a2a_skill
    fn list() -> Vec<AbilityDescriptor>;  // always the same logical type
}
```

The three abilities above are the three concrete implementations of
this trait. They differ only on:

1. **Trust / authentication domain**
   - local Invoke → caller authenticated by the local socket's
     filesystem permissions (mode 0600).
   - MCP bridge → caller authenticated by the MCP socket's
     filesystem permissions (same scheme; different socket because
     the MCP wire protocol is different).
   - A2A bridge → caller authenticated by the realm-wide identity
     attached to the discovery query (see `federation.resolve`).

2. **Wire shape**
   - Canonical: `{ abilities: AbilityDescriptor[] }` — every field
     of the descriptor present, including visibility / scope /
     hints / source.
   - MCP tool: `{ tools: [{ name, description, inputSchema }, ...] }`
     — three fields, MCP-spec-mandated. Other descriptor fields are
     dropped because the MCP client wouldn't know what to do with them.
   - A2A skill (v2 envelope): per-agent grouping, `runtime` instead
     of `kind`, no visibility (A2A is implicitly PUBLIC at this
     layer), `a2a_schema_version` per entry.

3. **Selection / routing implications**
   - All three are byte-identical for the "name" field — a router
     that selects an ability by name can use any plane interchangeably.
   - Visibility filtering applies at the provider's source: MCP
     bridge today projects PUBLIC + SCOPED-matching by default
     (`profile_local_private_for_owner` exception per §A5); A2A
     bridge projects everything advertised in `a2a.agents_json` —
     which today is PUBLIC + SCOPED reachable from the realm.
     Local Invoke returns the full local catalog and lets the
     admission gate filter by visibility per §1.6.

## The strict superset rule

The canonical shape (`meta.list_abilities`) is a **strict superset**
of every plane-specific shape. A planner that stays on the canonical
plane sees everything; a planner that consumes the MCP or A2A plane
sees a subset. Field renames are not allowed — `name` is `name`
across all three.

This is enforced today by:

- `mcp.bridge.list_tools` calls `tool_specs_from_descriptors(&[d])`
  in `runtime/agents/profiles/mcp.rs`. The function literally
  reads `d.name`, `d.schema_summary.input`, and a derived
  description; nothing else.
- `a2a.bridge.list_skills` calls
  `registry::a2a_labels::build_agents_envelope(&registry)`. The
  envelope's per-agent `skills` array is built from
  `runtime::abilities::abilities_for(name, &entry)`, whose elements
  carry `name` directly.
- `meta.list_abilities` returns `descriptors_provider()` verbatim
  via serde.

The "byte-identical name" property is the load-bearing invariant.
A future test could add a fixture cross-checking that the same
ability name appears in all three planes for a fixed registry; for
now the layer-classification test +
`description_for_and_input_schema_for_cover_every_published_name`
catch most drift.

## Downstream selection (planner / routing)

A planner that needs to dispatch an ability follows one rule:

> The ability name is the universal handle. Pick the plane based
> on which authentication / trust domain you have access to, not
> on which catalog you discovered the ability in.

Concretely:

- A local CLI tool dispatches via `~/.easynet/control.sock`
  regardless of whether it discovered the ability through
  `meta.list_abilities` or saw it advertised on the realm.
- An external MCP client (`claude code`) discovered the ability
  via `mcp.bridge.list_tools` and dispatches via the MCP socket
  using `tools/call`.
- An A2A peer discovered the ability via the realm directory and
  dispatches via signed `Invoke` over the realm's transport.

In all three cases the ability NAME is the same. The plane is a
transport detail, not part of the identity.

## Why this matters now

Three discovery planes that drift would fragment the planner. The
worst-case is "the ability shows up under one name in MCP and a
different name in the canonical view because someone added a
prefix in one plane and not the other." Locking the strict-superset
rule + the byte-identical-name invariant before more abilities land
keeps the planner story coherent.

The next planes that may show up:

- A2A `agents/list` over a federated transport (different wire
  channel, same agent-card envelope as `a2a.bridge.list_skills`).
- `federation.resolve` from the hub (different provenance — the
  hub's RealmDirectory rather than the local catalog — but same
  AbilityDescriptor shape per §1.2).

Both fit the unification cleanly. Document them when they land.
