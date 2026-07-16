# Agent Purge Publication FSM v1

## Status and use case

This is the normative internal contract for A38/A87 purge recovery. It covers
the concrete failure case in which `agent.purge` has committed local registry,
authority, identity, and filesystem removal, but the owner-projection
tombstone or Hub identity revoke has not yet completed.

The system must preserve three facts across every crash:

1. which immutable Agent incarnation was purged;
2. which publication stage is still required;
3. why automatic recovery stopped, if its finite budget was exhausted.

## Identity and ABA fence

An outbox entry binds one `transaction_id` to:

- logical Agent name and canonical Agent URA;
- owner-cursor `generation` (the immutable incarnation);
- authoritative host/authority URA;
- tombstone projection revision, digest, and exact payload;
- current stage, delivery fence, retry evidence, and reconciliation audit.

Retiring an owner cursor retains its generation high-water mark. Re-registering
the same URA MUST allocate `generation + 1` using checked arithmetic. A delayed
transaction for generation N MUST NOT remove inventory, directory rows,
ability rows, or presence belonging to generation N+1. Pending and
reconciliation-required entries retain the name and URA fence. No automatic
path may discard, acknowledge, or unfence them.

## Device outbox FSM

Publication stages are:

1. `tombstone_pending`;
2. `revoke_pending`;
3. completed, represented by atomic outbox removal.

Delivery states are:

- `ready`: zero failures in the current stage and eligible for claim;
- `claimed { drain_epoch, delivery_fence }`: owned by the sole bounded drain;
- `backing_off { eligible_drain_epoch }`: retry deferred by monotonic drain
  epochs, not wall-clock time;
- `reconciliation_required`: the finite per-stage attempt budget is exhausted.

Allowed transitions are:

```text
ready -> claimed
backing_off -> claimed
claimed -> backing_off
claimed -> reconciliation_required
claimed(tombstone_pending) -> claimed(revoke_pending)
claimed(revoke_pending) -> completed
reconciliation_required -> ready     [authorized audited Retry only]
```

Tombstone success resets the attempt budget before entering `revoke_pending`.
Each claim advances a persisted per-entry `delivery_fence` with checked
arithmetic. Attempt exhaustion enters `reconciliation_required`; scheduled,
connectivity-ready, and boot drains MUST NOT network-retry that state.

## Drain ownership and clock rollback

One dedicated cross-process drain guard covers at most one bounded network
batch. It is not the global lifecycle mutation guard, so local Agent lifecycle
mutations continue while network I/O is in flight. A concurrent drain that
cannot acquire the guard reports pending work and performs no remote call.

After acquiring the guard, recovery advances a persisted `drain_epoch` and
recovers any claim left by a crashed prior guard owner. Claim eligibility and
takeover never depend on an absolute Unix lease. Unix timestamps are evidence
only; clock rollback cannot expire a live owner or authorize takeover.

Every remote purge mutation carries the monotonic `delivery_fence`. The Hub
persists the highest fence for the transaction. A lower fence MUST fail before
mutation; an equal or higher fence with the same logical command may recover
or replay. This prevents a delayed slow worker from applying after takeover.

## Tombstone publication

Purge uses a projection-only publisher for the exact journaled empty ability
set. It MUST NOT call `federation.advertise_agent`. Publishing the tombstone
therefore cannot recreate identity inventory before revoke. Projection
generation, revision, and digest jointly fence stale or conflicting read-model
updates.

## Canonical revoke command

The logical revoke command contains exactly:

- protocol version;
- purge transaction ID;
- Agent URA;
- Agent generation;
- reason;
- authority URA;
- target URA.

The Hub computes a domain-separated SHA-256 digest over length-delimited
canonical fields and persists both command and digest. Reusing a transaction
ID with any changed logical field MUST fail closed. `delivery_fence` is a
transport-attempt fence and is persisted separately; advancing it does not
change the logical command digest.

## Hub durable revoke FSM

Hosted Agent inventory and revoke transactions share one locked, atomically
written durable repository. The process-local directory maps are projections,
never the source of truth.

Hub states are:

```text
Absent -> Prepared(command, digest, observed presence session, max fence)
Prepared -> Applied(exact outcome)
Applied -> Applied(exact outcome)      [idempotent replay]
```

Processing order is mandatory:

1. validate command, digest binding, and delivery fence;
2. persist `Prepared` with the exact command and observed session identity;
3. conditionally retire durable inventory only when URA, authority, and
   generation match;
4. persist `Applied` with one exact disposition: `retired`,
   `already_retired`, or `superseded_by_new_incarnation`;
5. compare-and-remove directory and ability projections for that generation;
6. remove direct presence only if its captured session ID still matches;
7. return success.

No success is recorded or returned before step 3 commits. Recovery of
`Prepared` repeats step 3 and completes `Applied`. Replay of `Applied` returns
the exact persisted outcome and may re-project compare-and-remove operations.
It MUST NOT interpret an absent row as ambiguous proof of prior success.

Registration serializes on the same repository. Equal generation with changed
facts is rejected, lower generation is stale, and only a strictly higher
generation may replace a retired incarnation. Thus an old Prepared revoke
observing a newer record completes as `superseded_by_new_incarnation` without
mutating it.

## Manual reconciliation

Manual reconciliation is an application operation requiring a typed
Manage-authorized proof. Its command contains a unique reconciliation command
ID, purge transaction ID, actor URA, and action. The command ID is durably bound
to the canonical command digest.

`Retry` is legal only from `reconciliation_required`. It resets the current
stage budget to `ready` while retaining transaction identity, generation,
stage, publication facts, delivery-fence high-water, terminal evidence, and an
immutable audit entry containing authorization reference and actor. Repeating
the same reconciliation command returns the same outcome without another
transition or audit append. Reusing its command ID with changed fields fails
closed. This specification defines no manual acknowledge or unfence action.

## Persisted-state validation

Every locked read and write rejects contradictory state, including:

- unsupported schema or duplicate transaction/name/URA identity;
- zero or exhausted generation, drain epoch, or delivery fence;
- attempts beyond the finite budget;
- `ready` with current failure evidence;
- `backing_off` without attempts, matching-stage evidence, or a finite
  recoverable drain epoch;
- `claimed` without claim ID, drain epoch, or a fence below the persisted
  next-fence high-water;
- `reconciliation_required` without terminal matching-stage evidence;
- changed revoke command under an existing transaction digest;
- changed reconciliation command under an existing command ID;
- `u64::MAX` sentinel timestamps or checked-arithmetic overflow.

Corruption is fail-closed. There is no legacy replay fallback.

## Required deterministic verification

Tests MUST cover poison isolation, stage-budget reset, dead-letter threshold and
no later automatic retry, retained identity fence, authorized reconciliation
and immutable replay audit, restart persistence, corrupt-state rejection,
Prepared-to-Applied crash recovery, command-digest conflict, old-transaction /
new-incarnation ABA replay, stale delivery fence after takeover, projection-only
tombstones, concurrent in-process and cross-process drains, and checked counter
overflow.
