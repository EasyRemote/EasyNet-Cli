# Ability layer model — semantic separation rules

## Why this exists

After the C-M9a / C-M10 / C-M13 work landed nine new abilities, the
surface risked reading as "a bag of RPCs." This doc articulates the
four semantic classes every published ability MUST belong to and
the invariants each layer guarantees. The invariants are
machine-checkable and lock the surface against drift.

## The semantic classes

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
| `chat.history.list` / `.get` | persisted chat-session JSONL (pure read) |
| `context.clipboard.list` / `.get` | persisted clipboard history (pure read) |
| `context.folders.list` / `context.fs.list` | mapped-folder table + contained dir listing |
| `context.favorites.list`   | persisted favorites file              |
| `context.captures.list` / `.get` | persisted media-capture index + payloads |

### 2. Control / decision (`consent.decide`; historical `policy.*`)

**Promise:** *decision logic is explicit about what it mutates. No
mutation of catalog state, no hidden state across calls.*

Decision abilities take a candidate envelope or permission ID and
return Allowed / Denied / Pending. The standalone `policy.evaluate`
and `policy.simulate` ability names are historical governance design
notes, not published EasyNet-Cli P0 abilities. If that product surface
is reintroduced later, it must be pure for identical inputs within the
decision TTL and must not mutate policy state.

Device-context configuration writes also live here:
`context.clipboard.track` (flip history capture on/off) and
`context.favorites.add` / `context.favorites.remove` (curate the
starred set). They mutate small operator-owned preference state, not
the ability catalog — the same decision-surface class as
`consent.decide`.

`consent.decide` is the borderline case: it does mutate the broker's
pending queue. That is allowed because the mutation IS the decision
being recorded — it's a write-only-after-decision channel, not a
side effect of the decision logic. The invariant is "no observation
ability's response changes because consent.decide ran." Verify with
`consent.list_pending` before/after.

| Ability / concept             | Current status | Mutation? |
|-------------------------------|----------------|-----------|
| `consent.decide`              | published      | broker queue (write-only after decision) |
| `policy.evaluate` / `.simulate` | historical design note | none if reintroduced |

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

### 4. Operational

**Promise:** *the ability invocation is the work.* These abilities may
read or mutate state, dispatch to a subprocess, call another ability, or
change an agent workspace. They are not coalesced as pure reads.

`meta.teach`, `meta.acquire`, and `meta.forget` live here: they mutate
the teach grant ledger or a learner agent workspace. They are not
introspection simply because they use the `meta.*` namespace.

| Ability family | Work performed |
|----------------|----------------|
| `meta.teach` / `meta.acquire` / `meta.forget` | capability transfer grant, learned manifest copy, learned-copy removal |
| `ability.*` / `skill.*` mutation verbs | write/remove manifests or skill package state |
| `fs.*`, `process.exec`, `shell.run`, `http.request` | host locomotion / execution work |
| `terminal.*`, `mic.*`, `camera.*`, `screen.*`, `speaker.*`, `voice.*`, `browser.*`, `mission.*` | session, media, browser, or orchestration work |
| `openai.chat_completions`, `openai.list_models`, `openai.files.*` | device-local OpenAI compatibility projection over host-local abilities and files |

## Cross-layer rules

- An introspection ability MUST NOT call a control ability.
  (Otherwise calling `meta.list_abilities` could trigger a
  policy.evaluate audit log entry — observation with side effects.)
- A control ability MAY call an introspection ability. For example, a
  future policy evaluator may fetch the descriptor catalog to score
  an envelope against descriptor visibility rules.
- An observation ability MUST NOT call any non-observation class.
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
