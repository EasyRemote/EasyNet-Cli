# Plan v4.1.3: Amendment to v4.1.2 (Naming Resolutions + C-M13 Audit Closeout)

## Status

v4.1.3 is an **amendment** to v4.1.2, not a full re-issue. It changes
only what is enumerated below. Every other section of v4.1.2 carries
forward unchanged.

After approval, v4.1.3 supersedes v4.1.2 only on the points listed
here; readers should hold v4.1.2 open as the base document and consult
v4.1.3 for the patched rows.

## Why this amendment

The C-M13 ontology audit (`AXON-RFC-001-c-m13-ontology-audit.md`)
surfaced three naming questions left explicitly open in v4.1.2:

| # | Topic | v4.1.2 status | v4.1.3 resolution |
|---|---|---|---|
| D1 | `<agent>.chat` vs `conversation.send` | both names existed; live CLI used `<agent>.chat`, §18 named `conversation.send` | **D1 below** — keep both, with documented mapping |
| D2 | `fleet.skill_install` vs `meta.acquire` | both verbs in §18, same shape, no rule on which to use | **D2 below** — `fleet.skill_install` is the operator verb; `meta.acquire` is the Agent-self-extension verb |
| D3 | `discuss.*` namespace | live in CLI, absent from §18 | **D3 below** — adopted into §18 as a first-class namespace |

In addition, v4.1.3 closes three housekeeping items:

| # | Topic | v4.1.2 status | v4.1.3 resolution |
|---|---|---|---|
| H1 | Realized abilities count vs §18 binding count | not tracked anywhere stable | **H1 below** — references the audit doc as authoritative |
| H2 | Bucket-C still-blocked abilities | listed only in audit doc | **H2 below** — pinned in §A so a future audit can't quietly drop them |
| H3 | `admission_internal` enforcement evidence | §A6 stated the rule; no test reference | **H3 below** — names the negative test that pins the rule |

---

## D1 — `<agent>.chat` and `conversation.send` coexist

**Resolution:** both names are valid. They name the *same wire-shape*
ability on the *same* llm-profile Agent. Choose by caller context:

- **`<agent>.chat`** is the **convenience-namespaced** form. The
  callee Agent's URA is encoded *into the ability name* (the prefix
  before `.chat` is the agent's local name, e.g. `claude.chat`,
  `codex.chat`). This shape exists because (a) early CLI users
  invoked agents by name without thinking in URAs, and (b) the
  per-agent manifest at `~/.easynet/agents/<name>/abilities/` already
  registers per-name handlers in axon-runtime — keeping the live name
  matches the on-disk publisher.

- **`conversation.send`** is the **canonical** form. The callee Agent
  is identified by the envelope's `callee` URA; the ability name is
  fixed. This is the form §18 names; it is the form a remote caller
  who already knows the URA should use.

**Rules:**

1. The CLI MUST register the per-agent shape (`<agent>.chat`) for every
   registered LLM agent. This is the form `easynet ability invoke
   claude.chat` and the chat handler in
   `runtime::agents::chat_ability` already uses.

2. The CLI SHOULD additionally register a `conversation.send` handler
   on each llm-profile Agent's local catalog, dispatching to the same
   underlying handler as `<agent>.chat`. Until that handler lands, the
   §19 "Frontend chat" row's wire form remains `<agent>.chat` — the
   backend translates the callee URA into the per-agent ability name
   when calling the daemon.

3. The §18 visibility rule (`conversation.*` defaults SCOPED to host
   operator) applies to **both** names. A daemon that registers
   `conversation.send` MUST inherit the same scope rule the per-agent
   `<agent>.chat` uses; a future visibility-promotion (`meta.publish`)
   applied to one applies to both.

4. `conversation.stream` is the streaming sibling — same dual-name
   rule.

§18 is amended to add the parenthetical `(also addressable as
<agent>.chat for the convenience name)` next to `conversation.send`
and `conversation.stream`.

§19 is amended: the `easynet ability invoke claude.skill.alive-video`
row is unchanged, but a parallel row is added for the canonical form:

| Function | Caller | Subject | Authority | Callee | Ability | Section |
|---|---|---|---|---|---|---|
| Frontend chat (canonical form, when registered) | backend-profile | operator | backend SessionAuthority | target llm-profile Agent | `conversation.send` (or stream) | §11 |
| Frontend chat (live convenience form, today) | backend-profile | operator | backend SessionAuthority | device-profile (proxy) | `<agent>.chat` per per-agent manifest | §11 |

---

## D2 — `fleet.skill_install` is the operator verb; `meta.acquire` is the Agent-self verb

**Resolution:** both names are valid; the §18 table keeps both. They
differ in **caller context**, not wire shape:

- **`fleet.skill_install`** is the **operator verb** addressed at the
  device-profile. The caller is the device-profile (acting on behalf
  of the local operator); the call adds a skill to a *named target
  Agent*. Today this is the live CLI form: `easynet skill install
  <source> --agent claude` invokes `fleet.skill_install` with
  `{name, source, target_agent}`.

- **`meta.acquire`** is the **Agent-self verb** addressed at the
  Agent itself. The caller IS the Agent acquiring the skill (or a
  delegate the Agent has authorized via DelegationProof). Args don't
  carry a `target_agent` because the target is the callee.

A given skill installation can be modeled either way:
- Operator-driven (`fleet.skill_install`): operator → device-profile
  → fans out to target Agent. Permission check at the device.
- Agent-driven (`meta.acquire`): Agent calls itself (or its host
  device-profile via internal delegation). Permission check at the
  Agent.

**Rules:**

1. v4.1.3 defines **no aliasing** between the two. A handler that
   wants to support both registers two distinct entries with the
   same underlying logic; the §A test pins this is intentional, not
   a name typo.

2. Live CLI ships `fleet.skill_install` / `_remove` / `_upgrade`
   today. `meta.acquire` / `meta.forget` are reserved for the future
   Agent-self path; not currently registered (Bucket-C, see H2).

3. The §18 table rows for both verbs are unchanged from v4.1.2.
   Their coexistence is now load-bearing, not accidental.

---

## D3 — `discuss.*` enters §18 as a first-class namespace

**Resolution:** the `discuss.*` ability set (currently `discuss.create`,
`discuss.post`, `discuss.subscribe`) is adopted into §18 on the
**device-profile**. It is not a subset of any existing namespace and
does not duplicate any §18 verb.

§18 is amended with these rows (insert in alphabetic order between
`consent.*` and `fleet.*`):

| Namespace | Ability | Owner profile | Default Visibility | Input | Receipt body |
|---|---|---|---|---|---|
| discuss | create   | device | SCOPED to local operator | `{topic, participants[]}` | `{discussion_id}` |
| discuss | post     | device | SCOPED to participants   | `{discussion_id, content}` | `{post_id, posted_at}` |
| discuss | subscribe| device | SCOPED to participants   | `{discussion_id}` | streaming `{post}` events |

Rationale:
- The CLI already has them; live tests already exercise them. Adopting
  them removes the audit-doc TODO without breaking anything.
- Owner profile is **device** (not its own profile) because the live
  `DiscussService` is a device-resident in-process service with no
  network endpoint of its own. A future profile split is possible
  (e.g. multi-host discussions through a hub-mediated room) but does
  not require changing §18 today; the URA stays on device-profile and
  any remote access goes through cross-device Invocation per §16.

§19 is amended with three corresponding rows for `easynet discuss
{create,post,subscribe}` mirroring the `easynet schedule add` /
`easynet loop create` patterns (operator → local-socket DelegationProof
→ device-profile → ability).

---

## H1 — Realized abilities are tracked in the C-M13 audit doc

**Resolution:** `docs/rfc/AXON-RFC-001-c-m13-ontology-audit.md` is the
authoritative live-vs-binding ability table. After every milestone
that lands a Bucket-A or Bucket-B ability, the audit doc's per-row
checkmark column MUST be updated in the same commit. CI does not
enforce the audit doc directly (it would race the live registry test);
instead, the existing
`runtime::agents::tests::description_for_and_input_schema_for_cover_every_published_name`
test backstops drift on the live side, and the audit doc backstops
drift on the spec side.

A future addition (out of scope for v4.1.3): a CI `warn` test that
reads §18 and the audit table and reports any binding-listed ability
without an audit-table row. Out of v4.1.3 because the warn surface
needs design work that this amendment does not block on.

---

## H2 — Bucket-C still-blocked abilities (pinned)

The following §18 abilities are deliberately not yet registered. v4.1.3
pins their reasons so a future audit cannot quietly skip them:

| Ability | Blocker | Tracked under |
|---|---|---|
| `mcp.bridge.call_tool` | Needs daemon `LocalRuntime` self-call through the Axon ability surface | C-M9a-ii |
| `mcp.client.list` / `mcp.client.call` | Needs MCP client library | C-M9b |
| `a2a.bridge.send_task` | Same registry self-reference issue as `mcp.bridge.call_tool` | C-M10-ii |
| `a2a.client.send_task` | Same as `mcp.client.*` (needs A2A client library) | C-M10-iii |
| `fleet.session_input` / `_read` / `_resize` / `_close` | Bidi infra not landed (`InvokeBidi` proto + kernel + FFI) | C-M3b/c, C-M1b |
| `meta.publish` | Needs visibility-mutation primitive + DirectoryEntry write surface | (untracked) |
| `meta.compose` | Needs ability-composition primitive | (untracked) |
| `meta.cancel` | Needs invocation-handle table | (untracked) |
| `admin.failover` / `.snapshot` / `.recover` | Multi-host HA story not yet specified | (untracked) |

§A — Ontology Consistency Checklist gains a new row:

| # | Rule | Verifier |
|---|---|---|
| A17 | Bucket-C abilities listed above remain unimplemented but in §18. A future revision MAY remove them from §18 only with an explicit superseding amendment; silent removal is forbidden. | This v4.1.3 H2 table |

---

## H3 — `admission_internal=true` negative-test evidence

**Resolution:** §A6 already states "the flag is never serializable from
untrusted sources." v4.1.3 names the verifier explicitly so a future
reader can find it without grep:

§A6 is amended with the trailing sentence: "**Verifier:** the
transport-layer drop test asserting an inbound envelope carrying
`admission_internal=true` is rejected before the admission gate runs.
Lives in `core/runtime-rs/src/runtime/admission_test.rs` (Axon repo)
and `internal/axon/admission_test.go` (Backend repo); the §20 `admission_internal=true from network` row references this."

(If those test files do not yet exist when v4.1.3 lands, this amendment
is the schedule note for them: they are required to exist before the
next plan revision can claim §A6 is enforced. The C-M14 closeout PR
should either reference the existing tests or land them.)

---

## Cross-repo coordination notes

v4.1.3 lands as a docs-only commit in EasyNet-Cli. The corresponding
work that *implements* the amendment lives in:

- **EasyNet-Cli**: register `conversation.send` / `conversation.stream`
  ability handlers per llm-profile Agent that dispatch to the same
  underlying chat handler as `<agent>.chat`. Out of scope for v4.1.3
  itself; tracked as a follow-up under C-M13 closeout.
- **EasyNet/backend**: the §19 "Frontend chat (canonical form)" row
  becomes the supported form once the CLI handlers exist; backend
  switches its outbound ability name when the daemon advertises both.
- **EasyNet-Axon**: no changes required — the protocol primitive is
  Invocation; ability names are payload, not protocol.

---

## Approval gate

After your approval of v4.1.3, the audit doc's "Decisions deferred to
user" section can be removed and the §18 + §19 entries amended above
become binding. If you confirm, please reply with "approved" or
specify which point needs further revision.
