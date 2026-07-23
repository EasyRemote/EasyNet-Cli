# Agent purge publication state canonicalization

## Goal

Remove the Hub-specific purge response compatibility projection from the daemon
agent lifecycle surface. `agent.purge` already emits the canonical generic
publication lifecycle fields: `publication_state` and `publication_error`.
Duplicating that state into `hub_tombstone_*` and `hub_revoke_*` keeps a
product-specific lifecycle model alive inside the runtime-owned agent lifecycle
ability.

## Boundary proof

- This slice changes only the `agent.purge` response projection.
- The canonical `publication_state` state machine remains unchanged:
  `not_applicable`, `reconciliation_required`, `pending`, `published`.
- The canonical `publication_error` evidence remains unchanged.
- Durable purge journal, publication outbox, owner cursor, tombstone/revoke
  execution and replay semantics remain unchanged.
- Removing Hub-specific response aliases converges the runtime surface toward
  product-neutral lifecycle naming.

## Invariants

1. `agent.purge` emits exactly one publication lifecycle projection.
2. The runtime-owned projection uses generic `publication_state` /
   `publication_error` naming.
3. No active agent lifecycle production code emits `hub_tombstone_state`,
   `hub_tombstone_error`, `hub_revoke_state`, or `hub_revoke_error`.
4. SPEC v2 rejects reintroduction of those Hub-specific compatibility fields.
5. Existing purge publication/outbox/recovery tests continue to validate the
   underlying lifecycle state machine.

## Verification plan

- Focused lifecycle tests covering purge publication state/outbox/recovery.
- SPEC v2 gate and self-test.
- Architecture convergence and formatting checks.
- codegraph sync/status after edits.

## Delta log

- Removed `hub_tombstone_state`, `hub_tombstone_error`, `hub_revoke_state`, and
  `hub_revoke_error` from `agent.purge` and `agent.stop` response projection.
- Replaced the previous two Hub-specific response states with one generic
  publication projection: `publication_state` plus `publication_error`.
- Preserved the underlying tombstone/revoke publication side effects, owner
  cursor updates, purge journal, outbox and recovery semantics.
- Added SPEC v2 structural and mutation coverage that rejects the retired
  Hub-specific response projection.

## Verification results

- `cargo test -q --features axon-pb unavailable_publisher_leaves_only_a_durable_outbox_without_quarantine_deadlock`
- `cargo test -q --features axon-pb backed_off_revoke_poison_does_not_block_later_purge_publication`
- `cargo test -q --features axon-pb corrupt_credentials_do_not_block_local_purge_or_publication_replay`
- `cargo test -q --features axon-pb outbox_ready_hook_redrives_publisher_waiting_purge`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `tools/scripts/check-architecture-convergence.sh`
- `cargo fmt --check`
- `git diff --check`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`
