# Ability layer model — semantic separation rules

## Why this exists

After the C-M9a / C-M10 / C-M13 work landed nine new abilities, the
surface risked reading as "a bag of RPCs." This doc articulates the
three semantic layers every published ability MUST belong to and
the invariants each layer guarantees. The invariants are
machine-checkable and lock the surface against drift.

## The three layers

### 1. Introspection (`meta.*`, `*.bridge.list_*`, `fleet.list_*`)

**Promise:** *pure, side-effect free, deterministic for a given
catalog snapshot.*

A caller may invoke any introspection ability arbitrary times, in
any order, and observe one of two things:
- The catalog hasn't changed → identical responses (modulo timestamps
  the caller doesn't depend on).
- The catalog *has* changed (skill installed, agent added) → the
  response reflects the new snapshot, but no introspection call
  caused that change.

| Ability                    | Source                                |
|----------------------------|---------------------------------------|
| `meta.describe`            | persisted local-agents.json + descriptors |
| `meta.list_abilities`      | live AbilityDescriptor catalog        |
| `mcp.bridge.list_tools`    | live AbilityDescriptor catalog (MCP-projection) |
| `a2a.bridge.list_skills`   | live AgentRegistry (A2A v2 envelope)  |
| `fleet.list_agents`        | live AgentRegistry (operational view) |
| `consent.list_pending`     | PermissionService::pending() snapshot |

### 2. Control / decision (`policy.*`, `consent.decide`)

**Promise:** *pure decision functions. No mutation of catalog
state, no hidden state across calls.*

Decision abilities take a candidate envelope (or a candidate
permission ID) and return Allowed / Denied / Pending. Calling
`policy.evaluate` for the same input twice yields the same decision
within the decision's TTL. They MAY consult persisted policy state,
but they MUST NOT mutate it (`policy.publish` is a separate verb,
intentionally absent from v1).

`consent.decide` is the borderline case: it does mutate the broker's
pending queue. That is allowed because the mutation IS the decision
being recorded — it's a write-only-after-decision channel, not a
side effect of the decision logic. The invariant is "no observation
ability's response changes because consent.decide ran." Verify with
`consent.list_pending` before/after.

| Ability                | Mutation? |
|------------------------|-----------|
| `policy.evaluate`      | none      |
| `policy.simulate`      | none (named distinctly to make it obvious) |
| `consent.decide`       | broker queue (write-only after decision) |

### 3. Observation (`observe.*`)

**Promise:** *derived state only. Calling an observe.* ability
never triggers behavior elsewhere.*

`observe.health` is a smoke test (returns daemon-side timestamp).
`observe.network_health` returns membership state derived from
local-agents.json. Neither makes a network call, neither modifies
state, neither schedules work.

The eventual-consistency caveat: `observe.network_health` reads
membership state that may be stale relative to a concurrent
`federation.heartbeat`. The response is **a snapshot**, not a live
view. A future enrichment that wires gRPC liveness probes will keep
the same shape but will bound staleness by a configurable TTL —
documented inline in the handler when that lands.

| Ability                  | Snapshot or live? |
|--------------------------|-------------------|
| `observe.health`         | per-call snapshot (timestamp from now) |
| `observe.network_health` | per-call snapshot of local-agents.json + hosted Agents |

## Cross-layer rules

- An introspection ability MUST NOT call a control ability.
  (Otherwise calling `meta.list_abilities` could trigger a
  policy.evaluate audit log entry — observation with side effects.)
- A control ability MAY call an introspection ability.
  (`policy.evaluate` legitimately fetches the descriptor catalog
  to score the envelope's ability against the descriptor.visibility
  rules.)
- An observation ability MUST NOT call either of the other two.
  (Otherwise observation can trigger admission, which is the
  textbook anti-pattern.)

## Invariant test

`runtime::agents::tests::ability_layer_classification_is_complete`
walks `published_ability_names()` and asserts each name maps to a
known layer. New abilities that don't fit any layer fail the test —
authors must either declare the layer or amend this doc.

## Bridges as a fourth-axis dimension, not a fourth layer

The MCP and A2A bridges are NOT a separate layer. They are
introspection-layer abilities with one extra property: their
*endpoint* is an edge protocol (MCP / A2A) rather than the local
Invoke socket. See `AXON-RFC-001-discovery-planes.md` for the
unification.
