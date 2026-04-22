# Spec — `<agent-root>/publish.json` Format

> **Status: SUPERSEDED** by `node-roster-label-v2.md` (2026-04-22, same PR-5a bundle).
>
> **Reason for supersession.** This spec designed a state machine
> (`pending | published | failed | partial`) for a dual-write *publish*
> operation — one half registering `AbilityToolAdapter` closures, the
> other half upserting an a2a label. That framing was inherited from
> `agent-publish-mechanism.md`, which itself is superseded: neither
> `AbilityToolAdapter::register` nor `a2a.agents_json` is an Agent-layer
> publish. AXIOM §6.2 locates real Agent publish at the Tier-2 discovery
> agent, which does not yet have a reserved URA (deferred in
> `DEFAULT_PROFILE.md`).
>
> **What replaces it.** Nothing, for now. The v1→v2 label flip
> (`node-roster-label-v2.md`) does not need a local state machine — the
> label is derived from `~/.easynet/agents.json` on every
> `register_node_with_options` call and has no independent lifecycle.
> When AXIOM §6.2 publish lands, the receipts it returns live in the
> Axon invocation log, not in a CLI-side JSON; a `publish.json`
> equivalent would be redundant with that chain.
>
> **Retraction.** The `--rollback` exit-code contract (0/1/2/3/4) and
> the six `agent doctor` checks were both built on a publish lifecycle
> that doesn't exist at the CLI layer. They are not load-bearing under
> the corrected scope and should not be carried forward.
>
> The original content follows unchanged. Do not build on it.

---

## Problem

`agent publish` must survive mid-flight failures. Two pieces of work happen during a live publish (mechanism per `agent-publish-mechanism.md`): registering each ability as an `AbilityToolAdapter` handler on the Axon bridge, and upserting the agent's entry into `a2a.agents_json` on the Axon node. If the CLI crashes, the bridge drops, or the label write fails, the operator needs to know *which step completed* so they can retry or back out without guessing.

`publish.json` is the on-disk record of publish intent + outcome, written per agent at `<agent-root>/publish.json`. It is *not* a replication log; it is the local source of truth for "what do I think I have published, and is that what Axon actually knows about me."

## File shape

```jsonc
{
  "schema_version": "1",
  "agent_name":     "alice",
  "state":          "published",      // see state machine below
  "published_at":   "2026-04-22T14:03:11Z",
  "last_error":     null,             // string when state is "failed" or "partial"
  "last_attempt_at": "2026-04-22T14:03:11Z",
  "abilities": [
    { "name": "alice.chat", "adapter_registered": true, "label_reflected": true }
  ],
  "axon_node_id":    "axon-node-abc123",   // whose a2a label we wrote into
  "a2a_schema_version": "v2"               // the label schema we produced
}
```

- `schema_version` — bumped only on a breaking change to this file shape. Readers refuse unknown versions.
- `agent_name` — redundant with the enclosing directory name but cheap to store; guards against an operator copying publish.json between agent roots by mistake.
- `state` — one of `pending | published | failed | partial`.
- `abilities[]` — one row per ability that was in scope for the publish; the two flags record whether each half of the dual-write succeeded.
- `axon_node_id` — the Axon node whose labels we mutated. If the CLI later connects through a *different* node (different machine, different tenant), publish.json is stale and `agent doctor` must flag it.
- `a2a_schema_version` — the label schema version we wrote. If a future release bumps the a2a schema, old `publish.json` records are still interpretable but prompt a re-publish.

## State machine

```
                     ┌─────────┐
     first attempt → │ pending │
                     └────┬────┘
          all steps ok    │
                          ▼
                    ┌───────────┐
                    │ published │ ◄──── `agent publish --retry` (if transitioning from failed/partial)
                    └─────┬─────┘
       remove / rollback  │
                          ▼
                     (file deleted)

     any step fails, none succeed → state = "failed", file persisted with last_error
     some succeed, some fail      → state = "partial", last_error names the failing step
     doctor sees partial/failed   → suggests `agent publish --retry` or `agent publish --rollback`
```

State transitions:

| From       | Event                                           | To          |
|------------|-------------------------------------------------|-------------|
| (no file)  | `agent publish` begins                          | `pending`   |
| `pending`  | all adapter registers + label upsert succeed    | `published` |
| `pending`  | no adapter registered, label not written        | `failed`    |
| `pending`  | at least one ability registered, then failure   | `partial`   |
| `failed`   | `--retry` succeeds                              | `published` |
| `partial`  | `--retry` succeeds                              | `published` |
| `published`| `agent remove`                                  | (deleted)   |
| `published`| operator edits `abilities/` and re-publishes    | `pending`→`published` (fresh attempt) |

### Why `partial` is its own state

A naive design would collapse `partial` and `failed` into one. We keep them separate because the rollback action differs:

- `failed` → nothing to undo on Axon; delete the file and retry when ready.
- `partial` → *some* adapter registrations are live on the bridge and *some* label entries are in place; `--rollback` must unregister each `abilities[*].adapter_registered == true` and remove the agent from the label.

Collapsing the two would force every rollback to do the full unregister dance defensively, which is both slower and noisier in the Axon audit log.

## Atomicity

- Writes: `persistence::config::atomic_write` (temp file + `rename`). No torn state visible to a concurrent reader.
- Permissions: inherit the directory's; no secrets in `publish.json` so 0o644 is fine.
- Backup: **none.** Unlike `agents.json` (which has a `.v1.bak` to survive a failed registry migration), `publish.json` is trivially recoverable from current Axon state + manifest list. `agent doctor` reconstructs the expected content and compares; mismatches are the diagnostic.

## `agent doctor` checks

For each publish.json encountered:

1. **Schema compatibility**: `schema_version` is in the supported set.
2. **File/agent parity**: `agent_name` matches the enclosing directory's AgentSpec name; refuse if they diverge (signals a cp/mv accident).
3. **Node parity**: CLI's current Axon node id equals `axon_node_id`; a mismatch is a warning, not an error (the operator may have moved machines).
4. **State ground truth**: if `state == "published"`, query the Axon bridge for each `abilities[*].name`; flag any ability not registered as `adapter_drift`. Query the a2a label; flag any ability not reflected as `label_drift`.
5. **Stale schema**: `a2a_schema_version` older than current; emit a nudge to re-publish.
6. **Pending stuck**: `state == "pending"` older than 30s; the CLI crashed mid-publish — suggest `--retry`.

Each check prints pass/warn/fail with a one-line remediation (`agent publish --retry`, `agent publish --rollback`, or "move machines → re-run `agent publish`").

## Why the file lives in the agent root

- Colocated with `agent.toml` and `abilities/`, so an operator who `rm -rf`'s the agent root cleans everything up in one shot.
- Doesn't pollute global state. Two machines can share the same `<agent-root>` over a mount / syncthing / git worktree and each maintain its own `axon_node_id` observation — wait, they can't. Two machines cannot share the same agent root safely because both would try to publish as the same identity. Document this as a hard constraint: *one agent root per machine*; sync layers must not replicate `publish.json` (or `abilities/`, since registration is per-node).

## Impact on PR-5b

- `publish/axon_handle.rs` owns the read/write of this file.
- `publish/ability_publish.rs` drives the state machine during the two-phase publish.
- `facade/cli/agent::run_doctor` gains the six checks above as a new section in its output.
- `agent remove` deletes `publish.json` after unregistering from Axon (order matters: unregister first so a crash mid-way leaves a `partial` file instead of an orphan adapter registration with no local record).

## `agent publish --rollback` exit codes

Scripts wrapping `agent publish --rollback` need a stable set of exit codes so they can distinguish "there was nothing to roll back" from "Axon was unreachable." PR-5b implements this; the contract is fixed here so the doctor checks above and the rollback command agree.

| Exit code | Meaning                                                             |
|-----------|---------------------------------------------------------------------|
| `0`       | Rollback successful: all registered adapters unregistered, a2a label entry removed, publish.json deleted. Also `0` when publish.json didn't exist (nothing to roll back; idempotent). |
| `1`       | Unexpected internal error (panic caught, OOM, unwritable disk for publish.json deletion). The bug-report case. |
| `2`       | Axon bridge unreachable. Rollback did nothing; caller should retry after reconnect. publish.json is untouched. |
| `3`       | `publish.json` is corrupt / unreadable. Rollback refuses to guess at state; caller must `--force` to unregister blindly against Axon and then delete the file manually. |
| `4`       | `publish.json` disagrees with Axon ground truth (e.g. state claims `published` but no adapter is registered). Rollback aborts to avoid compounding drift; caller runs `agent doctor` to see the divergence and decides whether to `--force`. |

`--force` semantics: bypass the publish.json read, query Axon for every `<agent>.<verb>` registered under the caller's node, unregister all matches, remove the agent from the a2a label, delete publish.json. Exit `0` on success, `2` if Axon is still unreachable, `1` on any other error. Never exits `3` or `4` under `--force`.

## Rollback plan for this spec

If PR-5b lands and we discover the state machine is wrong:

- `failed` / `partial` both collapse to "nothing useful on Axon" in the worst case → operators manually unpublish with `agent publish --rollback --force` (bypasses the local state read, queries Axon directly).
- `publish.json` is not cryptographic; delete it to force `agent doctor` to re-derive.

## Open question deferred

Whether `published_at` should carry a monotonic clock reading in addition to wall-clock time (for ordering across clock jumps) is deferred. Wall-clock is enough for human-readable diagnostics; Axon's own invocation receipts carry the monotonic piece.
