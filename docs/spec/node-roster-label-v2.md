# Spec — Node Roster Label (`a2a.agents_json`) v2

**Status:** Proposed · **Decision gate:** blocks PR-5b-relabel · **Owner:** Silan Hu · **Date:** 2026-04-22

## Scope — what this spec is, and is not

**This spec is:** the on-wire format of the `a2a.agents_json` string attached to an Axon node's `labels` map via `DendriteBridge::register_node_with_options(RegisterNodeOptions { labels })`. The label is a **node-level discovery hint** that the EasyNet backend parses to enumerate what agents a device is currently hosting, so the EasyNet Frontend can render them in its Agents list. The label travels as part of the standard Axon node descriptor — no invocation, no receipt, no chain, no signature beyond whatever the bridge applies to the whole node registration.

**This spec is not:**

- **Not an Agent-layer publish.** AXIOM §6.2 (Tier 2) locates real Agent publish at a reserved Tier-2 *discovery agent* that exposes `publish` / `unpublish` / `lookup` abilities. That agent's URA and ability signatures are marked `\deferred` in AXIOM pending `document/profiles/DEFAULT_PROFILE.md`. Writing a JSON blob onto a node label is **not** the same thing. When the discovery agent lands, its publish path will produce Invocation Axiom-conformant receipts; this label produces none.
- **Not a capability package publish.** `DendriteBridge::publish_capability` is the SDK surface for distributable capability packages (signed tar.gz artefacts with `payload_uri` / `package_bytes_base64` / `signature_fingerprint` fields), used for tenant-wide capability replication. An agent subprocess on the operator's machine is not a package and does not belong on that API.
- **Not authoritative for dispatch.** Incoming RPCs against `<agent>.<verb>` still flow through `AgentDispatchAdapter` on the CLI side (a process-local `AbilityToolAdapter::register` call, not a protocol-level publish). The label tells the Frontend what exists; the adapter makes it callable. The two must stay in sync — that's the load-bearing property, not the wire format of the label itself.

**Retirement.** This label has a retirement path that starts the moment AXIOM §6.2 discovery agent becomes implementable. Tracked in `../open-questions/retire-a2a-agents-json-label.md`.

## Problem this spec solves

Today (CLI v1.6.x writer + backend v1 parser) the `a2a.agents_json` label carries a bare JSON array `[{name, type, model, timeout, skills:[{skill_id, name, description, tags}]}]`. Two issues:

1. **Frontend agent rows only carry agent-level metadata** — there is no per-skill `input_schema` or per-skill timeout, so the Frontend cannot show an operator what arguments a `<agent>.chat` call expects. The `InvokeAbilityDialog` has to guess.
2. **v1 field names (`type`, `parameters`) drift from the AXIOM-aligned terminology** (`runtime`, `input_schema`) that every other layer uses. The drift is cosmetic but reader-hostile.

v2 is a one-shot rewrite to fix both. The label still exists only as a node-level hint; the rewrite does not extend its scope to "publish."

## Envelope

The `a2a.agents_json` label is **a JSON-encoded string** (Axon labels are strings) whose decoded value is a JSON object with one top-level key, `agents`:

```jsonc
{
  "agents": [
    { /* one agent entry */ },
    { /* another agent entry */ }
  ]
}
```

Rationale for wrapping in `{ "agents": [...] }` instead of a bare array:

- Leaves room for peer metadata (schema version, generator, timestamps) without breaking consumers.
- Matches the shape `node_mapper.go:ParseAgentsJSON` already expects (which today reads the v1 `{ "agents": [...] }` wrapper).

## Agent entry

Each entry:

```jsonc
{
  "name":       "alice",                   // AgentSpec.name; unique per node
  "runtime":    "claude-code",             // kebab-case RuntimeKind
  "model":      "claude-opus-4-7",         // string; optional (may be null or absent)
  "description": "code-review assistant",  // optional human blurb
  "a2a_schema_version": "v2",              // REQUIRED — see versioning below
  "skills": [
    {
      "name":             "alice.chat",    // fully-qualified, MUST equal `<name>.<verb>`
      "description":      "Send a chat prompt ...",
      "input_schema":     { /* JSON Schema, must be {"type":"object", ...} */ },
      "output_schema":    null,            // optional; null or absent means "opaque"
      "timeout_seconds":  null             // optional; null/absent means "runtime default"
    }
  ]
}
```

### Field rules

- `name`: `[a-z0-9][a-z0-9_-]*`; no dot (reserved for the `<agent>.<verb>` separator); no whitespace. Enforced by `AgentSpec::validate`.
- `runtime`: exactly one of `"claude-code" | "codex" | "codex-app-server"` as of v2. New runtimes are additive; a consumer MUST treat an unknown string as "runtime unavailable" and hide the agent from dispatch UX, not crash.
- `model`: free-form; MAY be `null` or absent. Consumers render a missing model as "default."
- `a2a_schema_version`: currently `"v2"`. Readers MUST reject an unknown string with a clear diagnostic; they MUST NOT silently downgrade. Both directions of drift (old reader, new writer *and* new reader, old writer) must be explicit.
- `skills[]`: zero or more. An empty array is legal (agent exists but offers no network tools yet).
- `skills[*].name`: **fully qualified** — `<agent.name>.<verb>`. Consumers MUST assert that the prefix equals `<agent.name>` before the first dot; a mismatch is the schema-violation signal this test is designed to catch (prevents a `publish` that accidentally routed `bob.chat` through `alice`'s agent record).
- `skills[*].input_schema`: REQUIRED. Must be a JSON object whose top-level `type` equals `"object"`.
- `skills[*].output_schema`: OPTIONAL. Same object-top-level constraint when present.
- `skills[*].timeout_seconds`: OPTIONAL `u64`. `null`/absent means "use the runtime default"; `0` is illegal (reject-at-write; see `AbilityManifest::validate`).

### Why both `name` on the agent AND fully-qualified name on the skill

The redundancy is deliberate. A consumer that reads a single skill line (e.g. in a log) has the full tool name without needing the enclosing context. A consumer that validates the skill array can assert the prefix matches the agent, catching a class of silent drift (mis-routing).

## Versioning

- Writer always stamps `a2a_schema_version: "v2"` on every entry it emits.
- Reader rejects any entry whose `a2a_schema_version` is not in the accepted set `{"v2"}`. No implicit v1 fallback; v1 data on the wire means "that peer has not upgraded" — consumers surface the version mismatch and hide the agent rather than render stale shape under a v2 contract.
- When v3 arrives: extend the accepted set, ship a reader that parses both, wait for the last writer to upgrade, then remove v2.

## Contract test (golden fixture)

`tests/fixtures/a2a-v2/golden.json` is the canonical byte-level document.

- CLI CI: asserts `registry::a2a_labels::build` produces *byte-equal* JSON given a fixed synthetic registry (helper: `build_golden_agents_label_for_testing`).
- EasyNet backend CI: asserts `node_mapper.ParseAgentsJSON` parses the fixture without error and extracts the expected shape.

Frontend has **no** CI coverage of this fixture by design — it consumes the backend's normalized `/api/v1/agents` response, not `a2a.agents_json`, so the wire shape is invisible to it. (An earlier draft listed a third Frontend CI here; that was written under the wrong assumption that the Frontend parsed the label directly.)

Neither CI is allowed to round-trip against the other's live service — a byte-stable fixture is the cheapest possible cross-stack contract.

### When the fixture must be updated

- Any v2 schema field addition: fixture updated, **two** CIs updated in the same release window (CLI writer + backend parser).
- Description text tweak: fixture updated, both CIs updated.
- Bug fix that changes output bytes for the same input: fixture updated, PR description explains the byte delta.

A PR that changes one end's parser or writer without updating the fixture MUST fail CI. That is the guarantee drift prevention relies on.

## Size / ordering rules

- Agents array MUST be sorted by `name` (ASCII). The byte-stable property depends on it.
- Skills array MUST be sorted by `name` (ASCII).
- No trailing whitespace; `serde_json` pretty print with 2-space indent. (Parsers must tolerate any valid JSON formatting; writers standardize for diff-friendly git history.)
- Total label size soft limit: **32 KiB**. A machine with hundreds of agents should split into multiple labels; not in scope for this spec. PR-5b SHOULD emit a stderr warning when the encoded label crosses **24 KiB** so the operator sees the approach to the limit before an outright split is forced.

## `null` vs absent for optional fields

Several fields are defined as "optional; MAY be `null` or absent." For byte-stable output, the writer MUST pick one and hold it:

- Writer emits `null` for every optional field that has no value. This keeps key order stable and makes the golden fixture a single canonical byte sequence.
- Reader tolerates either `null` or the key being absent. A peer running an older writer that omits the key is valid input.

`golden.json` reflects the writer rule (explicit `null`). A fixture that round-trips through the reader and back through the writer must remain byte-equal to `golden.json`.

## Breaking changes from current on-wire shape

Current `main` on both repos (EasyNet-Cli v1.6.x writer, EasyNet backend v1 parser) is on the v1 shape. There are no pre-v2 clients in the wild that we need to keep happy — EasyNet-Cli and the EasyNet backend are both single-owner repos under the same author, the federation has no external v1 consumers today, and the Frontend is insulated from the wire format (it consumes the backend's normalized `AgentInfo` response, not `a2a.agents_json` directly).

Given that, we do **not** ship tolerant parsers or dual-write fallbacks. We flip both ends to v2 in one release window. Earlier drafts of this spec called for a tolerant-parser-first upgrade path; that was written under the implicit assumption of cross-team coordination and is struck now that the repo topology is accurately understood.

| What is on the wire today (v1.6.x)                              | What ships at v2 flip day (this spec)                  |
|-----------------------------------------------------------------|--------------------------------------------------------|
| `a2a.agents_json` = bare JSON array `[...]`                     | Envelope `{ "agents": [...] }`                         |
| Agent entry carries `"type": "claude-code"`                     | Agent entry carries `"runtime": "claude-code"`         |
| Agent entry carries `"timeout": 300`                            | No agent-level `timeout` field (moved to per-skill)    |
| Skill carries `"parameters": { ... }`                           | Skill carries `"input_schema": { ... }`                |
| Skill missing `output_schema` and `timeout_seconds`             | Both fields present (value `null` when unset)          |
| Version carried at **label level**: `a2a.version = "2"`         | Version carried **per agent entry**: `a2a_schema_version = "v2"` (string, `v`-prefixed); the label-level `a2a.version` key is **removed**, not kept for co-existence |

### Files that change

EasyNet-Cli (PR-5b-relabel):
- `src/registry/a2a_labels.rs` — rewrite `build()` to emit the envelope shape and per-entry `a2a_schema_version`; remove the label-level `a2a.version` key.
- `src/runtime/abilities.rs::AgentAbilitySpec::to_discovery_json` — rename `parameters` to `input_schema`; add `output_schema` and `timeout_seconds` fields (both `null` for the seeded chat ability).
- `src/facade/cli/agent.rs::run_publish` + `summarize_schema` — dry-run table reads the renamed fields; user-facing column unchanged.
- Existing tests in `registry::a2a_labels` that assert on `a2a.version` label-level / `parameters` key / `type` agent-entry key — all rewritten.

EasyNet backend (companion PR, same release window):
- `backend/internal/axon/node_mapper.go::ParseAgentsJSON` — rewrite to expect the envelope. v1 bare-array input is explicitly rejected (returns `nil` with a log line, not silent fallback).
- `backend/internal/axon/client.go::AgentsJSONEntry` / `SkillInfo` — rename `Type` → `Runtime`; drop `Timeout`; rename skill `Parameters` field reference to `InputSchema`; add `OutputSchema` + `TimeoutSeconds`; add `A2ASchemaVersion` on the entry.
- `backend/internal/logic/agent/listAgentsLogic.go:58-65` — update the `e.Type` / `e.Model` reads (`e.Type` → `e.Runtime`; description / tag composition adjusts accordingly).
- `backend/internal/axon/real_helpers_test.go::TestParseAgentsJSON_*` — rewrite the v1-shape test fixture into the v2 envelope; add `TestParseAgentsJSON_RejectsBareArray` that asserts the no-fallback rule.

Frontend: **no changes**. `Frontend/src/lib/api/easynet-agents.ts` consumes `/api/v1/agents` (backend-normalized `AgentInfo`), not `a2a.agents_json` directly. The backend-side renames pass through its `axon.AgentInfo` → `Agent` mapping transparently.

### Cross-repo release order

Single-owner topology: both repos are yours, coordination is a git branch, not a calendar. Two PRs land on their respective `main` branches in one sitting:

1. **EasyNet backend** PR — rewrite parser + types + tests. Merges first.
2. **EasyNet-Cli** PR-5b-relabel — flip the label writer. Merges second.

Order matters only to keep the Frontend from rendering an empty agents list for the gap between the two merges. In practice the gap is minutes. No tolerant parser, no dual-write, no alternate label key — every one of those fallbacks would be paying the cost of cross-team coordination that doesn't apply.

The byte-for-byte contract between the two PRs is `tests/fixtures/a2a-v2/golden.json` in this repo, copied into `backend/internal/axon/testdata/a2a-v2-golden.json` during the backend PR. Both CIs assert against their own copy; a diff between the two fixture files is a three-way merge mistake and is caught by a one-line check script in `scripts/` that hashes both files.

### Cost estimate

- EasyNet-Cli PR-5b-relabel: ~80 lines Rust net (label writer rewrite in `a2a_labels.rs` + `to_discovery_json` field renames + test rewrites + `run_publish` dry-run text). No new modules, no new subsystem.
- EasyNet backend companion PR: ~150 lines Go (parser rewrite + type renames + test rewrite).

Neither PR is a "large change" by line count; the industrial-grade discipline is in the byte-stable fixture and the reject-v1 stance on the backend side, not in PR size.

## SDK compatibility

The label is written through `register_node_with_options(RegisterNodeOptions { labels })`, which has existed since well before the v1 label shape did. No SDK bump is required to flip the label format. The `easynet-axon` dependency stays at its current pin; the companion backend PR touches only its own parser, not its SDK import. (An earlier draft of this spec claimed a mandatory bump to SDK `1.2+`; that version number was fabricated — the real SDK is `easynet-axon 0.55.2` and nothing about it is load-bearing for this rewrite.)

## Impact on PR-5b-relabel

- `src/registry/a2a_labels.rs::build` rewrites to emit the v2 envelope + renamed fields.
- `src/runtime/abilities.rs::AgentAbilitySpec::to_discovery_json` renames the `parameters` key to `input_schema` and adds `output_schema` + `timeout_seconds` fields (both `null` for the seeded chat ability).
- **No new `publish/` module, no `publish.json`, no `agent publish` verb beyond the dry-run added in PR-4.** The `agent publish --dry-run` output gets updated to use the new field names; live publish stays unimplemented (and now correctly scoped to "registering abilities through the Axon discovery agent," not "writing a label," per the Open Question).
- Paired with one EasyNet backend companion PR (see "Files that change" above) that merges first. The byte-stable `tests/fixtures/a2a-v2/golden.json` is the only shared artefact. Frontend is untouched (consumes the backend's normalized `/api/v1/agents`).

## Related open questions

- `../open-questions/retire-a2a-agents-json-label.md` — when the AXIOM §6.2 discovery agent publish path lands, this label retires. The trigger conditions and retirement PR shape are tracked there.
- `../open-questions/axon-invocation-receipt-link.md` — whether CLI mission run artefacts should link to `invocation::Receipt`; unrelated to this label, cross-referenced only because a future consumer might want per-ability schema hashing there.
