# Agent Identity (L2)

**Status:** derived engineering artifact for ontology §6.4. Not part of
the ontology consensus. May evolve without an alignment session
provided it does not contradict ontology §6.4 (logical agent identity)
or §11.5 (deferred tenant model).

**Goal of this document:** constrain future errors. Not explain
design. If you read this and feel the urge to "improve" the model in
a way the constraints below forbid, stop and re-read the constraints.

---

## 1. The three layers

Agent identity exists at three layers in the EasyNet ecosystem.
The forms are intentionally **not** byte-equivalent.

| Layer | Form | Audience | Stability |
|---|---|---|---|
| **L1 Surface** | `silan/claude.chat(...)` | EAL users, CLI users | sugar, stable |
| **L2 Runtime** | `AgentId { tenant, name }` | Rust code, IR, dispatcher | this document |
| **L3 URA** | `easynet:///r/org/reg/agent.claude/abilities/chat` + envelope | wire protocol, conformance | future, see `../URA/README.md` |

L1 and L3 are parallel representations of the same logical agent.
L2 is the resolution layer between them. L1 is what humans write,
L3 is what the network speaks, L2 is what Rust code holds.

---

## 2. Strong constraints (non-negotiable)

### Constraint 1 — String non-equivalence

The three layer forms are not interchangeable strings. **Never**
construct one form by string-formatting another. In particular:

```rust
// FORBIDDEN
let uri = format!("easynet:///{}", agent_id);
let id  = uri.strip_prefix("easynet:///").unwrap();
```

L1 ↔ L2 ↔ L3 transitions go through **typed converters** (when L3
arrives) or through `AgentId::parse` / `Display` (for L1 ↔ L2 today).
Any code that does string concatenation between layers is broken.

Rationale: each layer has its own canonicalization rules.
L3 (URA) requires NFC normalization, percent-encoding, profile
selection, and signed bytes. L2 (`AgentId`) requires ASCII lowercase
and segment validation. L1 (EAL surface) is whatever the parser
accepts. They diverge as soon as anything non-trivial is involved.
A `format!` that ignores this is a silent ontology violation.

### Constraint 2 — Identity, not locator

`AgentId` answers **who**, never **where**. The struct contains
`tenant` and `name` and that is the entire field set, today and
forever. **Do not** add fields like:

```rust
// FORBIDDEN
struct AgentId {
    tenant: String,
    name: String,
    node_id: String,        // ❌ where the agent runs
    endpoint: String,       // ❌ how to reach it
    public_key: Vec<u8>,    // ❌ how to verify it
    instance: u64,          // ❌ which copy
}
```

`node_id` belongs on `IrTarget::Device`, not `AgentId`. Endpoints
belong on `AgentEntry` (the registry record). Public keys belong on a
future trust layer. Instances are reserved for ontology §6.4 v2 and
have their own form (see §6 below).

If you find yourself wanting to add a field to `AgentId`, the answer
is almost always: add it to `AgentEntry` instead, and look up the
entry by `AgentId`. The identity is the **key**, not the **value**.

Rationale: ontology §6.4 says agent logical identity is decoupled
from physical placement. Conflating `AgentId` with `node_id` /
`endpoint` reintroduces the exact mistake the ontology was written
to fix (interpretation A: device = degenerate agent).

---

## 3. AgentId definition

```rust
pub struct AgentId {
    pub tenant: String,
    pub name: String,
}
```

That is the complete struct. No other fields, ever. See Constraint 2.

### 3.1 Parse rules

`AgentId::parse(s: &str)` accepts two surface forms:

| Input | Result |
|---|---|
| `"claude"` | `AgentId { tenant: "default", name: "claude" }` |
| `"silan/claude"` | `AgentId { tenant: "silan", name: "claude" }` |

Both `tenant` and `name` segments must match `[a-z0-9_-]+` after
splitting on `/`. Specifically:

- ASCII only, lowercase only.
- Allowed characters: `a–z`, `0–9`, `_`, `-`.
- Each segment must be non-empty.
- Each segment must be at most 63 characters (DNS-style).

### 3.2 Rejection rules

The following inputs **must** be rejected with a parse error:

| Input | Reason |
|---|---|
| `""` | empty input |
| `"/"` | empty segments |
| `"claude/"` | empty name |
| `"/claude"` | empty tenant |
| `"a/b/c"` | multi-level namespaces reserved for v2 |
| `"claude#42"` | instance ids reserved for v2 (ontology §6.4) |
| `"Claude"` | uppercase forbidden |
| `"silan/Claude"` | uppercase forbidden |
| `"claude.chat"` | `.` not in charset (would conflict with EAL member-call) |
| `"claude/"` | trailing separator |
| `"审查员"` | non-ASCII forbidden |
| `"silan/claude#42"` | instance id forbidden |
| `"claude "` | whitespace forbidden |

No silent normalization. Wrong input → hard error. The L2 layer
exists to make wrong input visible early.

### 3.3 Equality rule

```rust
AgentId::parse("claude")? == AgentId::parse("default/claude")?  // true
```

This equality **must** hold. `claude` is shorthand for `default/claude`
and the two values are indistinguishable inside the runtime. Without
this rule, registry lookups silently miss and dispatch goes to the
wrong agent.

`Eq` and `Hash` are derived from the resolved (post-parse) `(tenant,
name)` pair, never from the input string. The input string is not
stored on `AgentId`.

### 3.4 Display rule

```rust
format!("{}", AgentId { tenant: "default".into(), name: "claude".into() })
// "default/claude"
```

`Display` always emits the **full form**, never the shorthand.
Storage and logs always see the canonical form. The shorthand is a
parse-time convenience only.

This is intentional: storage-runtime divergence is a class of bugs
that is impossible to debug after the fact. The file shows what the
runtime sees.

---

## 4. Where AgentId lives in the runtime

```rust
// L2 dispatch target — the type-safe replacement for the old
// stringly-typed `target_node_id: String` field.
pub enum IrTarget {
    Agent(AgentId),
    Device { node_id: String },
}

pub struct IrStep {
    pub target: IrTarget,
    pub ability: String,   // future: Ability { name, version }
    // ...
}
```

Dispatcher pattern (informational; the real impl is in
`src/eal/interpreter.rs`):

```rust
match &step.target {
    IrTarget::Agent(id)             => agent_registry.dispatch(id, ...),
    IrTarget::Device { node_id }    => bridge.dispatch(node_id, ...),
}
```

There is no `is_agent(name: &str)` string check anywhere in the
codebase. If you see one, it predates this document and should be
removed.

### 4.1 Registry storage

`~/.easynet/agents.json` stores keys in **full form**:

```json
{
  "agents": {
    "default/claude": { "agent_type": "claude-code", "...": "..." },
    "silan/codex":    { "agent_type": "codex",       "...": "..." }
  }
}
```

Load accepts both shorthand and full form (so existing files don't
break) but save always writes full form. There is no per-tenant
fall-through in storage. Storage matches runtime.

### 4.2 EAL surface

EAL accepts both forms in member-call position:

```eal
let r = claude.chat(prompt: "hi")        // tenant defaults to "default"
let r = silan/claude.chat(prompt: "hi")  // explicit tenant
```

The `/` is a lexer token (`Token::Slash`) and the parser accepts an
optional `tenant /` prefix at the start of a member-call. Both forms
lower to the same `IrTarget::Agent(AgentId)` IR shape; only the
`tenant` field differs.

---

## 5. Migration to URA (future, do not implement now)

When URA L3 lands, the mapping will be:

| L2 (`AgentId`) field | L3 (URA URI) location |
|---|---|
| `name` | subject-value, encoded as `agent.<name>` under `subject-type=reg` |
| `tenant` | envelope `tenant_id` (portable mode) **or** URI query `?tenant_id=<x>` (bound mode, ontology §11.5) |
| (none — ability is on `IrStep`) | resource-kind = `abilities`, resource-path = `<ability-name>` |
| (none — version is future) | `@<version-ref>` |

Concretely, an L2 step like:

```rust
IrStep {
    target: IrTarget::Agent(AgentId { tenant: "silan", name: "claude" }),
    ability: "chat",
}
```

will map to L3 wire form:

```
URI:      easynet:///r/org/reg/agent.claude/abilities/chat
envelope: { tenant_id: "silan", ... }
```

(under URA portable tenant mode, the default — see URA §7.1).

The mapping is **forward-only**. There is no requirement that the
URA URI round-trip back to a unique L2 `AgentId`; URA carries more
information (subject-type, scope, namespace, version) that L2 does
not represent. The right transition is:

```rust
// FUTURE — when URA module exists
fn agent_id_to_ura(id: &AgentId, ability: &str) -> ura::Uri { ... }
// There is intentionally no `ura_to_agent_id`. Going from L3 to L2
// is lossy. If you need to dispatch by URA URI, do it through the
// L3 dispatch path, not by trying to extract an AgentId.
```

---

## 6. Reserved for v2

The following forms are **rejected** by `AgentId::parse` today, but
the parser knows about them and emits a specific error message
mentioning the v2 reservation. This is so future code can flip a
flag and accept them without changing the error vocabulary.

| Reserved form | Reason | Where it goes |
|---|---|---|
| `claude#42` | instance id | ontology §6.4 v2: `<tenant>/<name>#<instance>` |
| `a/b/c` | nested namespaces | not currently planned; reserved to avoid binding |
| URA-shaped strings (`easynet://...`) | wrong layer | L3, not L2 |

When v2 of any of these arrives, **add a new struct or a new variant**,
do not extend `AgentId` in place. The `AgentId` struct shape is
frozen by Constraint 2.

---

## 7. Non-goals

This layer does not, and will not, do any of the following.
If a requirement points at one of these, it belongs in a different
layer or a future epic.

- **Not a URI parser.** That is L3 (URA).
- **Not a canonicalization spec.** L3 has its own normalization
  (NFC, percent-encoding, signed bytes). L2 only validates
  ASCII-lowercase shape.
- **Not a multi-tenant trust model.** Tenant in L2 is a name, not
  an authorization context. Authorization belongs to a future
  policy layer.
- **Not a federation directory.** Discovery of remote agents
  belongs to A2A / hub layers, not to this struct.
- **Not a key management system.** Agent identity here is logical,
  not cryptographic. Public keys belong on a trust layer.
- **Not a routing decision.** `AgentId` says who, not where or
  how. Routing decisions live in the dispatcher.
- **Not a backwards-compat surface.** The `agents.json` shorthand
  is a parse-time convenience for existing files. New code, new
  tests, and new examples always use the full form.

---

## 8. Test corpus expectations

The `AgentId` test module must cover at least:

1. Shorthand parses to default tenant.
2. Full form parses unchanged.
3. Equality holds across shorthand and full form.
4. Hash consistency with equality (HashMap lookup with shorthand
   key finds full-form entry and vice versa).
5. Each rejection rule from §3.2 has a test case.
6. Display always emits full form.
7. Round-trip: `parse(display(parse(s)?)?)? == parse(s)?` for all
   accepted inputs.
8. The reserved-form errors mention "v2" or "reserved" so they are
   distinguishable from generic parse errors.

If a test in `src/shared/agent_id.rs` does not map to one of the
above, it is over-testing the encoding and should be removed in
favor of a test that maps to a constraint here.

---

## 9. Pointers

- Ontology source: `docs/easynet_ontology.tex` §6.4 (logical
  identity), §11.5 (tenant deferral)
- L3 protocol: `../URA/README.md` §6 (`easynet` scheme), §7
  (tenant semantics)
- Implementation: `src/shared/agent_id.rs` (struct + parse +
  tests), `src/eal/ir.rs` (`IrTarget` enum), `src/eal/interpreter.rs`
  (dispatcher match)
- Registry: `src/shared/agents.rs` (load/save + storage format)

---

## 10. Why `AbilityName` is not a peer of `AgentId`

**This is the load-bearing anti-regression section. If you find
`AgentId` and `AbilityName` and feel the asymmetry is "incomplete",
read this section before writing any code.**

### 10.1 The temptation (and why it is wrong)

A future contributor encountering this codebase will see two types:

```rust
pub struct AgentId { tenant: String, name: String }   // parse, Display, Eq, Hash, registry-key
pub struct AbilityName(String);                       // newtype, validation only
```

The natural inference is:

> "This design is asymmetric. Let me 'fix' it by promoting
> `AbilityName` to a peer identity type, perhaps:
>
>     struct AbilityRef { agent: AgentId, name: String }
>     IrStep { target: AbilityRef }"

**This inference is wrong, and the resulting design is forbidden.**
The asymmetry is intentional. It encodes ontology §4 directly into
the type system. Removing the asymmetry collapses the OOP + RPC
hybrid model into a service-registry model and contradicts the
ontology.

### 10.2 The three hard prohibitions

#### Prohibition 1 — `AbilityName` is NOT an identity type

```text
AbilityName is NOT an identity type.

It must never be promoted to an addressable or routable entity.
```

`AbilityName` is a method name. It only has meaning in the context
of an `AgentId`. There is no `AbilityName::lookup`, no
`AbilityRegistry`, no canonical form, no equality across owners. Two
`AbilityName("chat")` values on different agents are different
methods that share a name — they are **not** the same ability.

`AbilityName` does not implement registry-key semantics. It does
not get a tenant. It does not get a hash-stable canonical form. It
is a `String` newtype with input validation, and that is the entire
contract.

#### Prohibition 2 — Ontology binding (Agent = WHO, Ability = WHICH METHOD)

```text
In EasyNet, the addressable unit is the Agent (ontology §6.4),
and the interaction unit is an ability call (ontology §4).

This implies:
- AgentId answers WHO
- AbilityName answers WHICH METHOD on that agent

There is no global "ability identity".
```

The phrase "EasyNet is the network of vocational abilities" in
ontology §2.2 is a statement about the **unit of network
interaction** — every cross-agent message is an ability call rather
than a raw RPC. It is **not** a statement that abilities are
globally addressable resources. The addressable unit remains the
agent. Ontology §4 (OOP view) makes this explicit: ability is a
public method, agent is the object that owns it.

The fact that an ability and its agent always travel together — and
that Alive's marketplace ranks `agent.ability` pairs — is direct
evidence of this binding. The pair is the interaction surface; it
is not evidence of a free-floating ability concept.

#### Prohibition 3 — Forbidden code pattern

```text
The following design is explicitly forbidden:

    struct AbilityRef { agent: AgentId, name: String }
    IrStep { target: AbilityRef }

This would turn the system into a service-registry model,
which contradicts the OOP + RPC hybrid defined in the ontology.
```

The service-registry path goes:

1. Promote `AbilityRef` to `IrStep.target`.
2. Add an `AbilityRegistry` that resolves `AbilityRef` to providers.
3. Routing becomes "find any agent that provides this ability",
   not "call this method on this agent".
4. The dispatcher starts doing capability matching across
   providers.
5. Ability becomes a free-floating resource; agent becomes a
   provider; the OOP model is gone.

Each step looks reasonable in isolation. The cumulative effect is
a different system. **Do not start down this path.** If a
requirement seems to demand this shape, the right response is to
re-read the ontology and find the correct model — not to extend
`AbilityName`.

### 10.3 What `AbilityName` *is*, in one sentence

> `AbilityName` is the second half of a two-part call path
> `(AgentId, AbilityName)`. It has no meaning alone. It is a typed
> string for input validation, not an identity layer.

`AbilityName` validates a slightly different character class from
`AgentId` segments: `[a-z0-9_.-]+` (note the `.`). Dotted ability
names are the existing EAL convention (`photo.capture`,
`health.check`). They are safe in `AbilityName` because ability names
appear only as values — never inside the EAL `agent.ability(...)`
member-call surface, which requires a single identifier and would
otherwise be ambiguous. To call a dotted ability, use the
traditional form `call "photo.capture" on "node"`.

### 10.4 Marketplace surface is a different layer

Alive's marketplace ranks `agent.ability` pairs (e.g.
`codex.review_pr ⭐4.8`). This is the **call path** as a product
unit — exactly the same `(AgentId, AbilityName)` pair the IR uses.
The marketplace renders it as `agent.ability` because that is what
users select. But selection is a presentation concern, not a
protocol concern.

The marketplace layer **does not** justify lifting `AbilityName`
into a peer identity. It justifies the existence of the
two-part call path, which is already in the IR as
`(IrTarget::Agent, ability: AbilityName)`.

### 10.5 If you still feel the asymmetry is wrong

Re-read §10.2 prohibition 3. Then re-read ontology §4 (OOP view)
and §2.2 (network of abilities). If you still feel the design is
incomplete, raise it as an alignment-session topic — do not unilaterally
introduce `AbilityRef`. The asymmetry is not a TODO; it is the design.
